#!/usr/bin/env python3
"""Machine-enforced stabilization-first development controller."""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import hmac
import json
import os
from pathlib import Path
import platform
import secrets
import signal
import stat
import subprocess
import sys
import tempfile
import time


ROOT = Path(__file__).resolve().parent.parent
PLAN_PATH = ROOT / "docs" / "stabilization-plan.json"
VERIFIER = ROOT / "scripts" / "stabilization_verifier.py"
PLAN_SCHEMA = "codeclew-stabilization-plan/1.0"


class ControlError(RuntimeError):
    pass


def canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")


def digest_bytes(value: bytes) -> str:
    return "sha256:" + hashlib.sha256(value).hexdigest()


def has_valid_embedded_digest(value: object, field: str) -> bool:
    if not isinstance(value, dict) or field not in value:
        return False
    expected = value[field]
    payload = dict(value)
    del payload[field]
    return expected == digest_bytes(canonical(payload))


def load_json(path: Path) -> object:
    with path.open("r", encoding="utf-8") as stream:
        return json.load(stream)


def atomic_private_write(path: Path, value: bytes, mode: int = 0o400) -> None:
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    os.chmod(path.parent, 0o700)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.{secrets.token_hex(8)}")
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(value)
            stream.flush()
            os.fsync(stream.fileno())
        os.chmod(temporary, mode)
        os.replace(temporary, path)
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass


def git(*arguments: str, check: bool = True) -> str:
    completed = subprocess.run(
        ("git", *arguments),
        cwd=ROOT,
        check=check,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
    )
    return completed.stdout.strip()


def validate_relative(value: str) -> None:
    path = Path(value)
    if not value or path.is_absolute() or ".." in path.parts or "\x00" in value:
        raise ControlError("plan contains an unsafe repository-relative path")
    if ".semantic-thread" in path.parts:
        raise ControlError("legacy state cannot be a stabilization input")


def validate_plan(plan: object) -> dict[str, object]:
    if not isinstance(plan, dict) or plan.get("schema") != PLAN_SCHEMA:
        raise ControlError("unsupported stabilization plan schema")
    if set(plan) != {"checks", "planId", "schema", "steps", "tiers"}:
        raise ControlError("stabilization plan fields differ from the closed schema")
    if not isinstance(plan["planId"], str) or not plan["planId"]:
        raise ControlError("planId is required")

    tiers: dict[str, dict[str, object]] = {}
    for tier in plan["tiers"]:
        if not isinstance(tier, dict) or set(tier) != {
            "budgetSeconds",
            "cleanRequired",
            "id",
            "minimumMemoryBytes",
            "minimumPhysicalCores",
        }:
            raise ControlError("invalid tier")
        tier_id = tier["id"]
        if tier_id in tiers:
            raise ControlError("duplicate tier")
        if not isinstance(tier["budgetSeconds"], int) or tier["budgetSeconds"] <= 0:
            raise ControlError("tier budget must be positive")
        if not isinstance(tier["cleanRequired"], bool):
            raise ControlError("tier cleanRequired must be boolean")
        for field in ("minimumMemoryBytes", "minimumPhysicalCores"):
            if not isinstance(tier[field], int) or tier[field] < 0:
                raise ControlError("invalid host qualification")
        tiers[tier_id] = tier
    if set(tiers) != {f"L{index}" for index in range(8)}:
        raise ControlError("tiers must define exactly L0 through L7")

    steps: dict[str, dict[str, object]] = {}
    order: list[str] = []
    for step in plan["steps"]:
        if not isinstance(step, dict) or set(step) != {"dependencies", "id", "requiredChecks"}:
            raise ControlError("invalid step")
        step_id = step["id"]
        if not isinstance(step_id, str) or not step_id or step_id in steps:
            raise ControlError("invalid or duplicate step")
        if not isinstance(step["dependencies"], list) or not isinstance(step["requiredChecks"], list) or not step["requiredChecks"]:
            raise ControlError("step dependencies/checks must be non-empty lists where required")
        steps[step_id] = step
        order.append(step_id)
    visited: set[str] = set()
    active: set[str] = set()

    def visit(step_id: str) -> None:
        if step_id in active:
            raise ControlError("step dependency cycle")
        if step_id in visited:
            return
        active.add(step_id)
        for dependency in steps[step_id]["dependencies"]:
            if dependency not in steps:
                raise ControlError("unknown step dependency")
            visit(dependency)
        active.remove(step_id)
        visited.add(step_id)

    for step_id in order:
        visit(step_id)

    checks: dict[str, dict[str, object]] = {}
    for check in plan["checks"]:
        if not isinstance(check, dict) or set(check) != {
            "command",
            "environmentKeys",
            "gate",
            "id",
            "inputRoots",
            "step",
            "tier",
        }:
            raise ControlError("invalid check")
        check_id = check["id"]
        if not isinstance(check_id, str) or not check_id or check_id in checks:
            raise ControlError("invalid or duplicate check")
        if check["step"] not in steps or check["tier"] not in tiers:
            raise ControlError("check references an unknown step or tier")
        if not isinstance(check["command"], list) or not check["command"] or not all(isinstance(value, str) and value for value in check["command"]):
            raise ControlError("check command must be a non-empty argv")
        if not isinstance(check["inputRoots"], list) or not check["inputRoots"]:
            raise ControlError("check input roots are required")
        for root in check["inputRoots"]:
            if not isinstance(root, str):
                raise ControlError("input root must be a string")
            validate_relative(root)
        if not isinstance(check["environmentKeys"], list) or not all(isinstance(value, str) and value for value in check["environmentKeys"]):
            raise ControlError("invalid environment key list")
        if check["gate"] is not None and (not isinstance(check["gate"], str) or not check["gate"]):
            raise ControlError("invalid gate identifier")
        checks[check_id] = check
    for step_id, step in steps.items():
        for check_id in step["requiredChecks"]:
            if check_id not in checks or checks[check_id]["step"] != step_id:
                raise ControlError("step requiredChecks authority mismatch")
    if set(checks) != {check for step in steps.values() for check in step["requiredChecks"]}:
        raise ControlError("every check must be required by exactly one step")
    return {"checks": checks, "order": order, "steps": steps, "tiers": tiers}


def state_root() -> Path:
    configured = os.environ.get("CODECLEW_CONTROL_HOME")
    root = Path(configured) if configured else Path.home() / ".cache" / "codeclew-control"
    if not root.is_absolute() or ".." in root.parts:
        raise ControlError("control home must be normalized and absolute")
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    metadata = root.stat()
    if metadata.st_uid != os.geteuid() or stat.S_IMODE(metadata.st_mode) & 0o077:
        raise ControlError("control home must be private and owner-controlled")
    os.chmod(root, 0o700)
    return root


def authorities(plan: dict[str, object]) -> dict[str, str]:
    return {
        "controllerDigest": digest_bytes(Path(__file__).read_bytes()),
        "planDigest": digest_bytes(canonical(plan)),
        "verifierDigest": digest_bytes(VERIFIER.read_bytes()),
    }


def selected_files(roots: list[str]) -> list[str]:
    arguments = ["ls-files", "--cached", "--others", "--exclude-standard", "-z", "--"]
    arguments.extend(roots)
    raw = subprocess.check_output(("git", *arguments), cwd=ROOT)
    paths = []
    for item in raw.split(b"\0"):
        if not item:
            continue
        value = item.decode("utf-8")
        if ".semantic-thread" not in Path(value).parts:
            paths.append(value)
    return sorted(set(paths))


def input_digest(check: dict[str, object]) -> str:
    hasher = hashlib.sha256()
    roots = list(check["inputRoots"])
    for argument in check["command"]:
        candidate = Path(argument)
        if (
            not argument.startswith("-")
            and not candidate.is_absolute()
            and "/" in argument
            and argument not in roots
        ):
            validate_relative(argument)
            roots.append(argument)
    for root in roots:
        path = ROOT / root
        if root != "." and not path.exists() and not path.is_symlink():
            hasher.update(b"missing\0" + root.encode("utf-8") + b"\0")
    for relative in selected_files(roots):
        path = ROOT / relative
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode):
            mode = b"symlink"
            data = os.readlink(path).encode("utf-8")
        elif stat.S_ISREG(metadata.st_mode):
            mode = b"executable" if metadata.st_mode & 0o111 else b"regular"
            data = path.read_bytes()
        else:
            raise ControlError("stabilization input is not a regular file or symlink")
        hasher.update(relative.encode("utf-8") + b"\0" + mode + b"\0")
        hasher.update(len(data).to_bytes(8, "big") + data)
    return "sha256:" + hasher.hexdigest()


def environment_digest(check: dict[str, object]) -> str:
    values = {key: os.environ.get(key) for key in check["environmentKeys"]}
    values["platform"] = {
        "machine": platform.machine(),
        "python": platform.python_version(),
        "system": platform.system(),
    }
    return digest_bytes(canonical(values))


def evidence_digest(check: dict[str, object], authority: dict[str, str]) -> str:
    value = {
        **authority,
        "clean": is_clean(),
        "commandDigest": digest_bytes(canonical(check["command"])),
        "environmentDigest": environment_digest(check),
        "memoryBytes": memory_bytes(),
        "physicalCores": physical_cores(),
        "sourceInputDigest": input_digest(check),
        "sourceRevision": git("rev-parse", "HEAD"),
    }
    return digest_bytes(canonical(value))


def physical_cores() -> int:
    if sys.platform == "darwin":
        try:
            return int(subprocess.check_output(("sysctl", "-n", "hw.physicalcpu"), text=True).strip())
        except (OSError, subprocess.SubprocessError, ValueError):
            return 0
    try:
        pairs: set[tuple[str, str]] = set()
        physical = core = None
        for line in Path("/proc/cpuinfo").read_text(encoding="ascii").splitlines() + [""]:
            if not line:
                if physical is not None and core is not None:
                    pairs.add((physical, core))
                physical = core = None
            elif line.startswith("physical id"):
                physical = line.split(":", 1)[1].strip()
            elif line.startswith("core id"):
                core = line.split(":", 1)[1].strip()
        return len(pairs)
    except OSError:
        return 0


def memory_bytes() -> int:
    if sys.platform == "darwin":
        try:
            return int(subprocess.check_output(("sysctl", "-n", "hw.memsize"), text=True).strip())
        except (OSError, subprocess.SubprocessError, ValueError):
            return 0
    try:
        for line in Path("/proc/meminfo").read_text(encoding="ascii").splitlines():
            if line.startswith("MemTotal:"):
                return int(line.split()[1]) * 1024
    except (OSError, ValueError, IndexError):
        pass
    return 0


def is_clean() -> bool:
    return not bool(git("status", "--porcelain=v1", "--untracked-files=all"))


def plan_state(authority: dict[str, str]) -> Path:
    return state_root() / "plans" / authority["planDigest"].split(":", 1)[1]


def completion_path(authority: dict[str, str], step: str) -> Path:
    return plan_state(authority) / "completions" / f"{step}.json"


def valid_completion(authority: dict[str, str], step: str) -> bool:
    path = completion_path(authority, step)
    try:
        value = load_json(path)
    except (FileNotFoundError, json.JSONDecodeError, OSError):
        return False
    return (
        isinstance(value, dict)
        and value.get("status") == "COMPLETE"
        and value.get("stepId") == step
        and has_valid_embedded_digest(value, "completionDigest")
        and all(value.get(key) == authority[key] for key in authority)
    )


def require_dependencies(model: dict[str, object], authority: dict[str, str], step: str) -> None:
    missing = [dependency for dependency in model["steps"][step]["dependencies"] if not valid_completion(authority, dependency)]
    if missing:
        raise ControlError("step prerequisites are incomplete: " + ",".join(missing))


def receipt_path(authority: dict[str, str], check: str, input_authority: str) -> Path:
    return plan_state(authority) / "checks" / check / f"{input_authority.split(':', 1)[1]}.json"


def exclusive_lock(authority: dict[str, str], name: str):
    path = plan_state(authority) / "locks" / f"{name}.lock"
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    os.chmod(path.parent, 0o700)
    stream = path.open("a+b")
    os.chmod(path, 0o600)
    fcntl.flock(stream, fcntl.LOCK_EX)
    return stream


def private_secret() -> bytes:
    path = state_root() / "capability.key"
    if not path.exists():
        atomic_private_write(path, secrets.token_bytes(32), mode=0o600)
    metadata = path.stat()
    if metadata.st_uid != os.geteuid() or stat.S_IMODE(metadata.st_mode) != 0o600:
        raise ControlError("capability key authority is invalid")
    value = path.read_bytes()
    if len(value) != 32:
        raise ControlError("capability key is corrupt")
    return value


def issue_capability(authority: dict[str, str], gate: str, budget_seconds: int) -> Path:
    payload = {
        "controllerDigest": authority["controllerDigest"],
        "expiresUnixMillis": int(time.time() * 1000) + (budget_seconds + 60) * 1000,
        "gate": gate,
        "nonce": secrets.token_hex(32),
        "planDigest": authority["planDigest"],
        "schema": "codeclew-stabilization-capability/1.0",
    }
    value = dict(payload)
    value["signature"] = hmac.new(private_secret(), canonical(payload), hashlib.sha256).hexdigest()
    path = state_root() / "capabilities" / f"{payload['nonce']}.json"
    atomic_private_write(path, canonical(value) + b"\n", mode=0o600)
    return path


def consume_capability(path: Path, gate: str, authority: dict[str, str]) -> None:
    if not path.is_absolute() or ".." in path.parts:
        raise ControlError("gate capability path is unsafe")
    metadata = path.stat()
    if metadata.st_uid != os.geteuid() or stat.S_IMODE(metadata.st_mode) != 0o600:
        raise ControlError("gate capability permissions are invalid")
    value = load_json(path)
    if not isinstance(value, dict) or set(value) != {
        "controllerDigest",
        "expiresUnixMillis",
        "gate",
        "nonce",
        "planDigest",
        "schema",
        "signature",
    }:
        raise ControlError("gate capability schema is invalid")
    signature = value.pop("signature")
    expected = hmac.new(private_secret(), canonical(value), hashlib.sha256).hexdigest()
    if not isinstance(signature, str) or not hmac.compare_digest(signature, expected):
        raise ControlError("gate capability signature is invalid")
    if value["gate"] != gate or value["planDigest"] != authority["planDigest"] or value["controllerDigest"] != authority["controllerDigest"]:
        raise ControlError("gate capability authority mismatch")
    if not isinstance(value["expiresUnixMillis"], int) or value["expiresUnixMillis"] < int(time.time() * 1000):
        raise ControlError("gate capability expired")
    used = path.with_suffix(".used")
    os.replace(path, used)


def file_digest(stream) -> str:
    stream.seek(0)
    hasher = hashlib.sha256()
    while True:
        block = stream.read(1024 * 1024)
        if not block:
            break
        hasher.update(block)
    return "sha256:" + hasher.hexdigest()


def invoke(check: dict[str, object], tier: dict[str, object], authority: dict[str, str]) -> tuple[int, int, str, str]:
    environment = dict(os.environ)
    capability: Path | None = None
    if check["gate"] is not None:
        capability = issue_capability(authority, check["gate"], tier["budgetSeconds"])
        environment["CODECLEW_PLAN_CAPABILITY"] = str(capability)
    started = time.monotonic_ns()
    try:
        with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
            try:
                process = subprocess.Popen(
                    check["command"],
                    cwd=ROOT,
                    env=environment,
                    stdin=subprocess.DEVNULL,
                    stdout=stdout,
                    stderr=stderr,
                    start_new_session=True,
                )
            except FileNotFoundError:
                exit_code = 127
            else:
                try:
                    exit_code = process.wait(timeout=tier["budgetSeconds"])
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGTERM)
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.wait()
                    exit_code = 124
                except BaseException:
                    try:
                        os.killpg(process.pid, signal.SIGTERM)
                        process.wait(timeout=5)
                    except (ProcessLookupError, subprocess.TimeoutExpired):
                        try:
                            os.killpg(process.pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                        process.wait()
                    raise
            duration = (time.monotonic_ns() - started) // 1_000_000
            stdout_digest = file_digest(stdout)
            stderr_digest = file_digest(stderr)
        return exit_code, duration, stdout_digest, stderr_digest
    finally:
        if capability is not None:
            for candidate in (capability, capability.with_suffix(".used")):
                try:
                    candidate.unlink()
                except FileNotFoundError:
                    pass


def verified_receipt(plan: dict[str, object], authority: dict[str, str], check: dict[str, object], tier: dict[str, object], input_authority: str) -> dict[str, object]:
    physical = physical_cores()
    memory = memory_bytes()
    clean = is_clean()
    qualified = physical >= tier["minimumPhysicalCores"] and memory >= tier["minimumMemoryBytes"]
    if not qualified or (tier["cleanRequired"] and not clean):
        exit_code, duration, stdout_digest, stderr_digest = 0, 0, digest_bytes(b""), digest_bytes(b"")
    else:
        exit_code, duration, stdout_digest, stderr_digest = invoke(check, tier, authority)
    request = {
        "checkId": check["id"],
        "clean": clean,
        "command": check["command"],
        "commandDigest": digest_bytes(canonical(check["command"])),
        "controllerDigest": authority["controllerDigest"],
        "durationMillis": duration,
        "environmentDigest": environment_digest(check),
        "exitCode": exit_code,
        "inputDigest": input_authority,
        "memoryBytes": memory,
        "physicalCores": physical,
        "planDigest": authority["planDigest"],
        "sourceRevision": git("rev-parse", "HEAD"),
        "stderrDigest": stderr_digest,
        "stdoutDigest": stdout_digest,
        "stepId": check["step"],
        "tier": check["tier"],
        "verifierDigest": authority["verifierDigest"],
    }
    with tempfile.NamedTemporaryFile(dir=state_root(), mode="wb", delete=False) as stream:
        request_path = Path(stream.name)
        stream.write(canonical(request) + b"\n")
    os.chmod(request_path, 0o600)
    try:
        completed = subprocess.run(
            (sys.executable, "-I", "-S", str(VERIFIER), "--request", str(request_path)),
            cwd=ROOT,
            check=False,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        if completed.returncode != 0:
            raise ControlError("independent verifier rejected the check evidence")
        value = json.loads(completed.stdout)
        if not isinstance(value, dict):
            raise ControlError("verifier returned an invalid receipt")
        return value
    finally:
        request_path.unlink(missing_ok=True)


def run_check(plan: dict[str, object], model: dict[str, object], authority: dict[str, str], step: str, check_id: str) -> dict[str, object]:
    if step not in model["steps"] or check_id not in model["checks"]:
        raise ControlError("unknown step or check")
    check = model["checks"][check_id]
    if check["step"] != step:
        raise ControlError("check does not belong to the requested step")
    require_dependencies(model, authority, step)
    input_authority = evidence_digest(check, authority)
    path = receipt_path(authority, check_id, input_authority)
    with exclusive_lock(authority, f"check-{check_id}-{input_authority.split(':', 1)[1]}"):
        if path.exists():
            existing = load_json(path)
            if (
                isinstance(existing, dict)
                and existing.get("status") == "PASS"
                and has_valid_embedded_digest(existing, "receiptDigest")
                and all(existing.get(key) == authority[key] for key in authority)
            ):
                return {"checkId": check_id, "reused": True, "status": "PASS"}
            raise ControlError("blind retry refused for the same failed evidence key")
        receipt = verified_receipt(plan, authority, check, model["tiers"][check["tier"]], input_authority)
        atomic_private_write(path, canonical(receipt) + b"\n")
        return {"checkId": check_id, "reused": False, "status": receipt["status"]}


def seal_step(model: dict[str, object], authority: dict[str, str], step: str) -> dict[str, object]:
    if step not in model["steps"]:
        raise ControlError("unknown step")
    require_dependencies(model, authority, step)
    receipt_digests = []
    for check_id in model["steps"][step]["requiredChecks"]:
        check = model["checks"][check_id]
        path = receipt_path(authority, check_id, evidence_digest(check, authority))
        if not path.exists():
            raise ControlError("required check has no receipt: " + check_id)
        receipt = load_json(path)
        if not isinstance(receipt, dict) or receipt.get("status") != "PASS":
            raise ControlError("required check did not pass: " + check_id)
        if not has_valid_embedded_digest(receipt, "receiptDigest"):
            raise ControlError("required check receipt integrity failed")
        if any(receipt.get(key) != authority[key] for key in authority):
            raise ControlError("required check authority is stale")
        receipt_digests.append(receipt["receiptDigest"])
    with exclusive_lock(authority, f"completion-{step}"):
        completion: dict[str, object] = {
            **authority,
            "receiptDigests": sorted(receipt_digests),
            "schema": "codeclew-stabilization-step-completion/1.0",
            "sourceRevision": git("rev-parse", "HEAD"),
            "status": "COMPLETE",
            "stepId": step,
        }
        completion["completionDigest"] = digest_bytes(canonical(completion))
        atomic_private_write(completion_path(authority, step), canonical(completion) + b"\n")
    return {"status": "COMPLETE", "stepId": step}


def status(model: dict[str, object], authority: dict[str, str]) -> dict[str, object]:
    completed = [step for step in model["order"] if valid_completion(authority, step)]
    next_step = next((step for step in model["order"] if step not in completed and all(dependency in completed for dependency in model["steps"][step]["dependencies"])), None)
    return {
        "completed": completed,
        "nextStep": next_step,
        "planDigest": authority["planDigest"],
        "schema": "codeclew-stabilization-status/1.0",
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("validate")
    subparsers.add_parser("status")
    run_parser = subparsers.add_parser("run")
    run_parser.add_argument("--step", required=True)
    run_parser.add_argument("--check", required=True)
    seal_parser = subparsers.add_parser("seal")
    seal_parser.add_argument("--step", required=True)
    guard_parser = subparsers.add_parser("guard")
    guard_parser.add_argument("--gate", required=True)
    arguments = parser.parse_args()
    try:
        plan = load_json(PLAN_PATH)
        assert isinstance(plan, dict)
        model = validate_plan(plan)
        authority = authorities(plan)
        if arguments.command == "validate":
            result = {"planDigest": authority["planDigest"], "schema": "codeclew-stabilization-plan-validation/1.0", "status": "PASS"}
        elif arguments.command == "status":
            result = status(model, authority)
        elif arguments.command == "run":
            result = run_check(plan, model, authority, arguments.step, arguments.check)
        elif arguments.command == "seal":
            result = seal_step(model, authority, arguments.step)
        else:
            capability = os.environ.get("CODECLEW_PLAN_CAPABILITY")
            if not capability:
                raise ControlError("direct expensive gate execution is forbidden")
            consume_capability(Path(capability), arguments.gate, authority)
            result = {"gate": arguments.gate, "schema": "codeclew-stabilization-gate-admission/1.0", "status": "ADMITTED"}
    except (AssertionError, ControlError, json.JSONDecodeError, OSError, subprocess.SubprocessError, ValueError) as error:
        print(canonical({"error": str(error), "schema": "codeclew-stabilization-control-error/1.0"}).decode("utf-8"))
        return 2
    print(canonical(result).decode("utf-8"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
