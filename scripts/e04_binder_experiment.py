#!/usr/bin/env python3
"""Frozen three-arm E04 binder-only experiment controller (stdlib only).

`run` never reads controller manifests. `judge` is the separate phase that
does, after every model run has been retained.
"""

from __future__ import annotations

import argparse
import contextlib
import hashlib
import io
import json
import os
import re
import shlex
import shutil
import statistics
import subprocess
import sys
import tempfile
import threading
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from typing import Any

BASE = "a6ae1e48359eccef15060c1bb249a648857f30c9"
POP_SHA = "a209f115b0a175bb74859b0539f75932cd664a495332ccf10b634b3cf1c2b9f2"
MODEL = "gpt-5.6-terra"
EFFORT = "low"
CODEX_VERSION = "codex-cli 0.147.0"
AST_VERSION = "ast-index 3.48.1"
BINDER_TREE_SHA = "fc349a728c92750e7eb36c39368ef693d708c98badccf4eb9c0a246279474ba4"
ARMS = ("default", "ast-index", "codeclew")
PUBLIC_KEYS = {
    "schema", "taskId", "buildSystem", "kotlinVersion", "task", "repository",
    "sourceSnapshotSha256", "buildCommand", "controllerManifestCommitment",
}
FAMILIES = (
    "producer-transform-consumer", "type-signature-propagation",
    "dto-event-api-evolution", "persistence-nullability",
    "configuration-lifecycle", "error-retry-resource",
    "test-regression-strengthening",
)
ROLES = (
    "CONTEXT_PRODUCER", "TRANSFORMER", "VALUE_EDGE", "DECLARATION", "OVERRIDE",
    "CALL_SITE", "CONTRACT_FIELD", "CONSTRUCTION_SITE", "COMPATIBILITY_POLICY",
    "PROJECTION", "DECLARED_TYPE", "QUERY_CONSUMER", "CONFIGURATION_PRODUCER",
    "INITIALIZATION_SITE", "LIFECYCLE_OWNER", "FAILURE_PATH", "RESOURCE_OWNER",
    "RETRY_OPERATION", "BEHAVIOR_UNDER_TEST", "INDEPENDENT_ORACLE",
    "PRODUCTION_CONTRACT",
)
REFUSALS = (
    "UNSUPPORTED_FAMILY", "MULTIPLE_COMPATIBLE_BINDINGS",
    "UNKNOWN_EFFECT_OR_LIFECYCLE", "EXTERNAL_POLICY_ABSENT",
    "SCHEMA_EVIDENCE_ABSENT", "QUERY_UNSUPPORTED",
    "UNRESOLVED_FRAMEWORK_BOUNDARY", "EXTERNAL_RETRY_POLICY_ABSENT",
    "UNKNOWN_SUSPEND_OR_EFFECT", "BUSINESS_ORACLE_ABSENT",
    "SELF_CONFIRMING_ORACLE", "INCOMPLETE_SEMANTIC_EVIDENCE", "PARTIAL_BUDGET",
)
FAMILY_CONTRACTS = {
    "producer-transform-consumer": {
        "roles": ["CONTEXT_PRODUCER", "TRANSFORMER", "VALUE_EDGE"],
        "obligations": ["bind producer and consumer", "preserve value-flow contract", "prove transform placement"],
    },
    "type-signature-propagation": {
        "roles": ["DECLARATION", "OVERRIDE", "CALL_SITE"],
        "obligations": ["propagate declared type", "preserve override compatibility", "preserve call-site assignability"],
    },
    "dto-event-api-evolution": {
        "roles": ["CONTRACT_FIELD", "CONSTRUCTION_SITE", "COMPATIBILITY_POLICY"],
        "obligations": ["update contract field", "propagate construction sites", "preserve compatibility policy"],
    },
    "persistence-nullability": {
        "roles": ["PROJECTION", "DECLARED_TYPE", "QUERY_CONSUMER"],
        "obligations": ["align projection and declared type", "preserve nullability", "link query result to consumer"],
    },
    "configuration-lifecycle": {
        "roles": ["CONFIGURATION_PRODUCER", "INITIALIZATION_SITE", "LIFECYCLE_OWNER"],
        "obligations": ["bind configuration producer", "preserve initialization order", "respect lifecycle region"],
    },
    "error-retry-resource": {
        "roles": ["FAILURE_PATH", "RESOURCE_OWNER", "RETRY_OPERATION"],
        "obligations": ["preserve failure path", "preserve resource closure", "preserve retry cardinality and order"],
    },
    "test-regression-strengthening": {
        "roles": ["BEHAVIOR_UNDER_TEST", "INDEPENDENT_ORACLE", "PRODUCTION_CONTRACT"],
        "obligations": ["identify independent oracle", "detect omitted behavior", "preserve production contract"],
    },
}
STATE_DIRECTORIES = {".e04-state", ".semantic-thread", ".gradle", "build", "target"}
RUNS_LOCK = threading.Lock()

ROOT = Path(__file__).resolve().parents[1]
POPULATION = ROOT / "benchmarks/semantic-change/editing-population-v1.json"
OUTPUT_SCHEMA = ROOT / "benchmarks/semantic-change/e04-model-output.schema.json"
FREEZE_MANIFEST = ROOT / "benchmarks/semantic-change/e04-freeze.json"
CORPUS_FILES = (
    ROOT / "crates/semantic-corpus/src/lib.rs",
    ROOT / "crates/semantic-corpus/src/main.rs",
    ROOT / "crates/semantic-corpus/src/e04.rs",
)


def compact(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def sha_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha_file(path: Path) -> str:
    return sha_bytes(path.read_bytes())


def load(path: Path) -> Any:
    with path.open(encoding="utf-8") as handle:
        return json.load(handle)


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    temporary.replace(path)


def append_jsonl(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as handle:
        handle.write(compact(value) + "\n")
        handle.flush()
        os.fsync(handle.fileno())


def command_output(args: list[str], cwd: Path = ROOT) -> str:
    result = subprocess.run(args, cwd=cwd, text=True, capture_output=True, check=False)
    if result.returncode:
        raise RuntimeError(f"command failed ({' '.join(args)}): {result.stderr.strip()}")
    return result.stdout.strip()


def population() -> dict[str, Any]:
    if sha_file(POPULATION) != POP_SHA:
        raise RuntimeError("frozen population digest mismatch")
    value = load(POPULATION)
    if value.get("plannedTaskCount") != 42 or len(value.get("slots", [])) != 42:
        raise RuntimeError("frozen population must contain exactly 42 slots")
    return value


def binder_paths() -> list[str]:
    paths = command_output(["git", "ls-tree", "-r", "--name-only", BASE]).splitlines()
    selected = []
    for path in paths:
        if path in {"Cargo.toml", "Cargo.lock", "build.gradle.kts", "settings.gradle.kts"}:
            selected.append(path)
        elif path == "crates/clew/Cargo.toml" or path.startswith("crates/clew/src/"):
            selected.append(path)
        elif path.startswith("schemas/"):
            selected.append(path)
        elif path == "workers/kotlin/build.gradle.kts" or path.startswith("workers/kotlin/src/main/"):
            selected.append(path)
        elif path == "workers/kotlin21/build.gradle.kts":
            selected.append(path)
    if len(selected) != 44:
        raise RuntimeError(f"frozen binder contour changed: expected 44 paths, found {len(selected)}")
    return sorted(selected)


def binder_tree_sha256() -> str:
    digest = hashlib.sha256()
    for path in binder_paths():
        data = subprocess.run(
            ["git", "show", f"{BASE}:{path}"], cwd=ROOT, capture_output=True, check=True
        ).stdout
        digest.update(path.encode())
        digest.update(b"\0")
        digest.update(data)
        digest.update(b"\0")
    return digest.hexdigest()


def obligations_catalog(spec: dict[str, Any]) -> tuple[str, ...]:
    return tuple(sorted({item for family in spec["families"] for item in family["requiredObligations"]}))


def common_prompt(spec: dict[str, Any]) -> str:
    catalog = {
        "familyContracts": FAMILY_CONTRACTS,
        "refusalCodes": list(REFUSALS),
        "oracleClasses": ["DERIVED", "PARAMETRIC", "MODEL_AUTHORED", "EXTERNAL_SPEC"],
    }
    return (
        "E04 is a binder-only experiment. Do not create, edit, delete, format, or build source. "
        "Return exactly one JSON object accepted by semantic-editing-e04-model-output/0.1. "
        "Infer only from the public task and permitted evidence. BOUND requires exactly one complete "
        "binding for every role of the inferred family, the exact three catalog obligations, evidence "
        "anchors that name each bound symbol, and an explicit oracle class. Never guess: return "
        "AMBIGUOUS with every complete binding set when two or more equally valid sets remain; return "
        "REFUSED with the most specific catalog code when evidence, policy, lifecycle/effect knowledge, "
        "query/schema support, or an independent oracle is missing. A tool being unsupported is not by "
        "itself evidence that the source task is semantically unsupported. "
        "The following global catalog is shared unchanged by every task and reveals no current "
        f"slot label:\n{compact(catalog)}"
    )


def mode_prompt(arm: str, clew: Path | None) -> str:
    if arm == "default":
        return "Use ordinary read-only filesystem search and exact source reads. Do not use ast-index or Codeclew."
    if arm == "ast-index":
        return "Use ast-index 3.48.1 for every navigation decision. Exact source reads at locations returned by ast-index are allowed; grep, rg, find, broad cat, and Codeclew are forbidden."
    binary = str(clew.resolve()) if clew else "<ABSOLUTE_CLEW_BINARY>"
    return (
        f"Use only the absolute binary {binary} with `prove map-edge-with-context`, `projection`, "
        "or `agent-context`/`context`. Try the public proof operation on the task-named target. "
        "Do not use filesystem source reads, grep/search, ast-index, or any other command. "
        "If the public operation cannot prove the task, return REFUSED."
    )


def validate_public(value: Any, path: Path) -> None:
    if not isinstance(value, dict) or set(value) != PUBLIC_KEYS:
        raise RuntimeError(f"invalid public manifest keys: {path}")
    if value["schema"] != "semantic-editing-e04-public-task/0.1":
        raise RuntimeError(f"invalid public schema: {path}")
    if value["repository"] != "repository" or not isinstance(value["task"], str):
        raise RuntimeError(f"invalid public repository/task: {path}")
    if not re.fullmatch(r"e04-[0-9a-f]{16}", value["taskId"]):
        raise RuntimeError(f"invalid opaque task ID: {path}")


def discover_public(experiment: Path) -> list[tuple[Path, dict[str, Any]]]:
    found = []
    for path in sorted((experiment / "agent").glob("*/task-manifest.json")):
        value = load(path)
        validate_public(value, path)
        repo = path.parent / value["repository"]
        if not repo.is_dir():
            raise RuntimeError(f"public repository missing: {repo}")
        found.append((path, value))
    if len(found) != 42 or len({value["taskId"] for _, value in found}) != 42:
        raise RuntimeError(f"E04 requires 42 unique public tasks, found {len(found)}")
    return found


def frozen_checks(check_tools: bool = True) -> dict[str, Any]:
    spec = population()
    if sha_file(OUTPUT_SCHEMA) == "":
        raise RuntimeError("output schema missing")
    head = command_output(["git", "rev-parse", "HEAD"])
    binder_digest = binder_tree_sha256()
    if binder_digest != BINDER_TREE_SHA:
        raise RuntimeError(f"binder tree mismatch: expected {BINDER_TREE_SHA}, got {binder_digest}")
    result = {
        "harnessCommit": head,
        "productBaseCommit": BASE,
        "binderTreeSha256": binder_digest,
        "populationSha256": POP_SHA,
        "outputSchemaSha256": sha_file(OUTPUT_SCHEMA),
        "commonPromptSha256": sha_bytes(common_prompt(spec).encode()),
        "plannedTasks": 42,
        "plannedRuns": 126,
        "model": MODEL,
        "reasoning": EFFORT,
    }
    if check_tools:
        codex = command_output(["codex", "--version"])
        ast = command_output(["ast-index", "--version"])
        if codex != CODEX_VERSION or ast != AST_VERSION:
            raise RuntimeError(f"tool freeze mismatch: codex={codex!r}, ast-index={ast!r}")
        result.update(codexVersion=codex, astIndexVersion=ast)
    if FREEZE_MANIFEST.is_file():
        manifest = load(FREEZE_MANIFEST)
        expected = {
            "productBaseCommit": BASE,
            "binderTreeSha256": binder_digest,
            "populationSha256": POP_SHA,
            "model": MODEL,
            "reasoning": EFFORT,
            "outputSchemaSha256": result["outputSchemaSha256"],
            "commonPromptSha256": result["commonPromptSha256"],
            "runnerSha256": sha_file(Path(__file__)),
            "corpusFileSha256": {str(path.relative_to(ROOT)): sha_file(path) for path in CORPUS_FILES},
        }
        if manifest.get("schema") != "semantic-editing-e04-freeze/0.1":
            raise RuntimeError("invalid E04 freeze manifest schema")
        for key, value in expected.items():
            if manifest.get(key) != value:
                raise RuntimeError(f"E04 freeze manifest mismatch: {key}")
        result["freezeManifestSha256"] = sha_file(FREEZE_MANIFEST)
        result["harnessCommit"] = manifest["harnessCommit"]
        result["freezeState"] = "FROZEN"
    else:
        result["freezeState"] = "PENDING_MANIFEST"
    return result


def slot_id(slot: dict[str, Any], index: int) -> str:
    raw = f"{index}:{slot['family']}:{slot['variant']}:{slot['buildSystem']}:{slot['ordinal']}"
    return "unmaterialized-" + sha_bytes(raw.encode())[:16]


def matrix(experiment: Path | None) -> list[dict[str, Any]]:
    spec = population()
    if experiment:
        tasks = [(value["taskId"], sha_file(path), str(path)) for path, value in discover_public(experiment)]
    else:
        tasks = [(slot_id(slot, i), None, None) for i, slot in enumerate(spec["slots"])]
    rows = []
    for task_id, manifest_sha, manifest_path in tasks:
        arm_order = sorted(ARMS, key=lambda arm: sha_bytes(f"{POP_SHA}:{task_id}:{arm}".encode()))
        for order, arm in enumerate(arm_order):
            rows.append({
                "runId": f"{task_id}--{arm}", "taskId": task_id, "arm": arm,
                "taskArmOrder": order, "publicManifest": manifest_path,
                "publicManifestSha256": manifest_sha, "state": "PLANNED",
            })
    if len(rows) != 126:
        raise RuntimeError("run matrix is not 42x3")
    return sorted(rows, key=lambda row: (row["taskId"], row["taskArmOrder"]))


def plan_packets(output: Path, experiment: Path | None, check_tools: bool) -> dict[str, Any]:
    freeze = frozen_checks(check_tools)
    rows = matrix(experiment)
    output.mkdir(parents=True, exist_ok=True)
    for row in rows:
        write_json(output / "planned" / row["runId"] / "run-packet.json", row)
    manifest = {
        "schema": "semantic-editing-e04-plan/0.1", "freeze": freeze,
        "experimentRoot": str(experiment.resolve()) if experiment else None,
        "runs": rows,
    }
    write_json(output / "plan.json", manifest)
    return manifest


def source_digest(root: Path) -> str:
    digest = hashlib.sha256()
    entries = sorted(
        ((path.relative_to(root).as_posix(), path) for path in root.rglob("*")),
        key=lambda entry: entry[0],
    )
    for relative_text, path in entries:
        relative_path = path.relative_to(root)
        if any(part in STATE_DIRECTORIES for part in relative_path.parts):
            continue
        if path.is_symlink():
            raise RuntimeError(f"source snapshot contains a symlink: {relative_path}")
        if not path.is_file():
            continue
        relative = relative_text.encode()
        digest.update(relative); digest.update(b"\0")
        digest.update(path.read_bytes()); digest.update(b"\0")
    return digest.hexdigest()


def event_metrics(lines: list[str]) -> tuple[dict[str, Any], list[dict[str, Any]], list[str]]:
    events, errors = [], []
    input_tokens = cached = output_tokens = 0
    native = False; turns = 0; tool_items: dict[str, dict[str, Any]] = {}; tool_bytes = 0
    for number, line in enumerate(lines, 1):
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            errors.append(f"INVALID_JSONL_LINE:{number}"); continue
        events.append(event)
        kind = event.get("type")
        if kind == "turn.completed":
            turns += 1; usage = event.get("usage") or {}
            def token(name: str) -> int:
                return int(usage.get(name, usage.get(''.join([name.split('_')[0]] + [p.title() for p in name.split('_')[1:]]), 0)) or 0)
            input_tokens += token("input_tokens"); cached += token("cached_input_tokens"); output_tokens += token("output_tokens"); native = bool(usage) or native
        item = event.get("item") if isinstance(event.get("item"), dict) else None
        if item and item.get("type") in {"command_execution", "mcp_tool_call", "web_search", "file_change"}:
            key = str(item.get("id", f"{number}:{item.get('type')}")); tool_items[key] = item
            for field in ("aggregated_output", "output", "result"):
                if field in item:
                    tool_bytes += len((item[field] if isinstance(item[field], str) else compact(item[field])).encode())
    commands = []
    for item in tool_items.values():
        if item.get("type") == "command_execution":
            command = item.get("command", "")
            rendered = " ".join(command) if isinstance(command, list) else str(command)
            output = ""
            for field in ("aggregated_output", "output", "result"):
                if field in item:
                    output += item[field] if isinstance(item[field], str) else compact(item[field])
            commands.append({"command": rendered, "output": output})
        elif item.get("type") == "file_change":
            commands.append({"command": "FILE_CHANGE_EVENT", "output": ""})
    metrics = {
        "turns": turns, "actionCalls": len(tool_items), "toolOutputBytes": tool_bytes,
        "inputTokens": input_tokens if native else None,
        "cachedInputTokens": cached if native else None,
        "outputTokens": output_tokens if native else None,
        "noncachedTokens": input_tokens - cached + output_tokens if native else None,
        "nativeTokenTelemetryAvailable": native,
    }
    return metrics, events, commands + [{"command": error, "output": ""} for error in errors]


def audit(
    arm: str,
    commands: list[Any],
    before: str,
    after: str,
    clew_path: Path | None = None,
    repository: Path | None = None,
) -> tuple[list[str], int]:
    flags = []
    if before != after:
        flags.append("SOURCE_MUTATION")
    navigation = 0; used_ast = False; used_clew = False
    edit = re.compile(r"(^|[;&| ])(apply_patch|rm|mv|cp|tee)([ ;&|]|$)|sed\s+-i|(^|[^>])>(?!>)")
    search = re.compile(r"(^|[ /])(rg|grep|find|fd|awk|cat|less)([ ;&|]|$)")
    ast_evidence = ""
    for record in commands:
        command = record if isinstance(record, str) else str(record.get("command", ""))
        command_output_text = "" if isinstance(record, str) else str(record.get("output", ""))
        if command.startswith("INVALID_JSONL_LINE:"):
            flags.append(command); continue
        if command == "FILE_CHANGE_EVENT":
            flags.append("SOURCE_EDIT_ATTEMPT"); continue
        lower = command.lower()
        if edit.search(lower): flags.append("SOURCE_EDIT_ATTEMPT")
        if re.search(r"\b(ast-index|rg|grep|find|fd|sed|cat|less|clew)\b", lower): navigation += 1
        if arm == "default" and ("ast-index" in lower or re.search(r"(^|/)clew\b", lower)):
            flags.append("DISALLOWED_MODE_TOOL")
        elif arm == "ast-index":
            if re.search(r"[;&|<>`\n]|\$\(", command):
                flags.append("FALLBACK_SEARCH"); continue
            try:
                tokens = shlex.split(command)
            except ValueError:
                flags.append("FALLBACK_SEARCH"); continue
            executable = Path(tokens[0]).name if tokens else ""
            if executable == "ast-index":
                used_ast = True
                ast_evidence += "\n" + command_output_text
            elif executable == "sed" and len(tokens) == 4 and tokens[1] == "-n" and re.fullmatch(r"\d+(?:,\d+)?p", tokens[2]):
                numbers = [int(value) for value in tokens[2][:-1].split(",")]
                span = (numbers[-1] - numbers[0] + 1) if len(numbers) == 2 else 1
                candidate = Path(tokens[3])
                if repository is None:
                    flags.append("FALLBACK_SEARCH")
                else:
                    resolved = (repository / candidate).resolve() if not candidate.is_absolute() else candidate.resolve()
                    relative = resolved.relative_to(repository.resolve()).as_posix() if resolved.is_relative_to(repository.resolve()) else ""
                    if span > 240 or not relative or not resolved.is_file() or relative not in ast_evidence:
                        flags.append("FALLBACK_SEARCH")
            else:
                flags.append("FALLBACK_SEARCH")
        elif arm == "codeclew":
            if re.search(r"[;&|<>`\n]|\$\(", command):
                flags.append("FALLBACK_SEARCH"); continue
            try:
                tokens = shlex.split(command)
            except ValueError:
                flags.append("FALLBACK_SEARCH"); continue
            expected = str(clew_path.resolve()) if clew_path else None
            if expected is None or not tokens or tokens[0] != expected:
                flags.append("FALLBACK_SEARCH"); continue
            position = 1
            if len(tokens) > position and tokens[position] == "--json":
                position += 1
            subcommand = tokens[position] if len(tokens) > position else ""
            if subcommand in {"projection", "agent-context", "context"}:
                used_clew = True
            elif subcommand == "prove" and len(tokens) > position + 1 and tokens[position + 1] == "map-edge-with-context":
                used_clew = True
            else:
                flags.append("FALLBACK_SEARCH")
    if arm == "ast-index" and not used_ast: flags.append("AST_INDEX_NOT_USED")
    if arm == "codeclew" and not used_clew: flags.append("CODECLEW_PROOF_NOT_USED")
    return sorted(set(flags)), navigation


def validate_binding(value: Any) -> bool:
    return isinstance(value, dict) and set(value) == {"role", "symbol"} and isinstance(value["role"], str) and isinstance(value["symbol"], str) and value["role"] in ROLES and bool(value["symbol"])


def validate_model_output(value: Any) -> list[str]:
    errors = []
    keys = {"schema", "status", "inferredFamily", "goal", "ambiguity", "refusal"}
    if not isinstance(value, dict) or set(value) != keys: return ["MODEL_OUTPUT_KEYS"]
    if value["schema"] != "semantic-editing-e04-model-output/0.1": errors.append("MODEL_OUTPUT_SCHEMA")
    if value["inferredFamily"] not in FAMILIES + ("UNKNOWN",): errors.append("INFERRED_FAMILY")
    status = value["status"]
    if status == "BOUND":
        goal = value["goal"]
        if value["ambiguity"] is not None or value["refusal"] is not None: errors.append("BOUND_EXCLUSIVITY")
        if not isinstance(goal, dict) or set(goal) != {"bindings", "obligations", "evidenceAnchors", "oracleClass"}: errors.append("BOUND_GOAL")
        elif not goal["bindings"] or not all(validate_binding(v) for v in goal["bindings"]) or not goal["obligations"] or not all(isinstance(v, str) and v for v in goal["obligations"]) or not goal["evidenceAnchors"] or not all(isinstance(v, str) and v for v in goal["evidenceAnchors"]) or goal["oracleClass"] not in {"DERIVED", "PARAMETRIC", "MODEL_AUTHORED", "EXTERNAL_SPEC"}: errors.append("BOUND_CONTENT")
    elif status == "AMBIGUOUS":
        ambiguity = value["ambiguity"]
        if value["goal"] is not None or value["refusal"] is not None: errors.append("AMBIGUOUS_EXCLUSIVITY")
        if not isinstance(ambiguity, dict) or set(ambiguity) != {"choices"} or not 2 <= len(ambiguity.get("choices", [])) <= 8: errors.append("AMBIGUITY")
        elif not all(isinstance(c, dict) and set(c) == {"bindings"} and c["bindings"] and all(validate_binding(v) for v in c["bindings"]) for c in ambiguity["choices"]): errors.append("AMBIGUITY_CHOICES")
    elif status == "REFUSED":
        refusal = value["refusal"]
        if value["goal"] is not None or value["ambiguity"] is not None: errors.append("REFUSED_EXCLUSIVITY")
        if not isinstance(refusal, dict) or set(refusal) != {"code"} or refusal["code"] not in REFUSALS: errors.append("REFUSAL")
    else: errors.append("MODEL_STATUS")
    return errors


def task_prompt(spec: dict[str, Any], public: dict[str, Any], arm: str, clew: Path | None) -> str:
    safe = {key: public[key] for key in PUBLIC_KEYS if key != "controllerManifestCommitment"}
    return f"{common_prompt(spec)}\n\nARM POLICY:\n{mode_prompt(arm, clew)}\n\nPUBLIC TASK:\n{compact(safe)}"


def execute_one(row: dict[str, Any], experiment: Path, output: Path, clew: Path) -> dict[str, Any]:
    public_path = Path(row["publicManifest"]); public = load(public_path); spec = population()
    run_dir = output / "runs" / row["runId"]; run_dir.mkdir(parents=True, exist_ok=True)
    prompt = task_prompt(spec, public, row["arm"], clew); (run_dir / "prompt.txt").write_text(prompt, encoding="utf-8")
    with tempfile.TemporaryDirectory(prefix="codeclew-e04-") as temporary:
        isolated = Path(temporary) / "repository"; shutil.copytree(public_path.parent / "repository", isolated, symlinks=True)
        before = source_digest(isolated)
        if before != public["sourceSnapshotSha256"]: raise RuntimeError(f"public source snapshot mismatch for {row['taskId']}")
        last = run_dir / "last-message.json"; events_path = run_dir / "events.jsonl"; stderr_path = run_dir / "stderr.txt"
        command = ["codex", "exec", "--ephemeral", "--ignore-user-config", "--skip-git-repo-check", "--json", "--output-schema", str(OUTPUT_SCHEMA), "-s", "workspace-write", "-m", MODEL, "-c", 'model_reasoning_effort="low"', "-C", str(isolated), "-o", str(last), "-"]
        environment = os.environ.copy()
        state = isolated / ".e04-state"; state.mkdir()
        environment["AST_INDEX_DB_PATH"] = str(state / "ast-index.db")
        started = time.monotonic(); process = subprocess.run(command, input=prompt, text=True, capture_output=True, check=False, env=environment); wall = int((time.monotonic() - started) * 1000)
        events_path.write_text(process.stdout, encoding="utf-8"); stderr_path.write_text(process.stderr, encoding="utf-8"); after = source_digest(isolated)
    lines = process.stdout.splitlines(); metrics, _, commands = event_metrics(lines); flags, navigation = audit(row["arm"], commands, before, after, clew, isolated)
    model_output = None; output_errors = []
    try: model_output = load(last); output_errors = validate_model_output(model_output)
    except Exception as error: output_errors = [f"MODEL_OUTPUT_UNREADABLE:{type(error).__name__}"]
    flags.extend(output_errors)
    goal_bytes = len(compact(model_output["goal"]).encode()) if isinstance(model_output, dict) and model_output.get("goal") is not None else 0
    packet = {**row, "state": "FINISHED", "exitCode": process.returncode, "executionStatus": "OK" if process.returncode == 0 and not output_errors else "FAILED", "wallMilliseconds": wall, "promptBytes": len(prompt.encode()), "contextBytes": len(prompt.encode()) + metrics["toolOutputBytes"], "goalBytes": goal_bytes, "navigationCalls": navigation, "auditFlags": sorted(set(flags)), "metrics": metrics, "modelOutput": model_output, "artifacts": {"eventsJsonl": str(events_path), "stderr": str(stderr_path), "lastMessage": str(last)}, "sourceBeforeSha256": before, "sourceAfterSha256": after}
    write_json(run_dir / "run-packet.json", packet)
    return packet


def run_all(args: argparse.Namespace) -> None:
    experiment = Path(args.experiment_root) if args.experiment_root else None
    output = Path(args.output)
    plan = plan_packets(output, experiment, True)
    if args.dry_run:
        print(compact({"status": "DRY_RUN", "plannedRuns": len(plan["runs"]), "output": str(output)})); return
    if experiment is None: raise RuntimeError("live run requires --experiment-root")
    clew = Path(args.codeclew_bin or "")
    if not clew.is_absolute() or not clew.is_file(): raise RuntimeError("--codeclew-bin must be an existing absolute frozen binary")
    if not FREEZE_MANIFEST.is_file(): raise RuntimeError("live run requires the committed E04 freeze manifest")
    if sha_file(clew) != load(FREEZE_MANIFEST).get("codeclewBinarySha256"):
        raise RuntimeError("Codeclew binary does not match the E04 freeze manifest")
    results_path = output / "runs.jsonl"
    existing = [json.loads(line) for line in results_path.read_text(encoding="utf-8").splitlines() if line] if results_path.exists() else []
    existing_ids = {row["runId"] for row in existing}
    if len(existing_ids) != len(existing): raise RuntimeError("duplicate retained run IDs")
    pending = [row for row in plan["runs"] if row["runId"] not in existing_ids]
    workers = max(1, min(int(args.max_workers), 4))
    with ThreadPoolExecutor(max_workers=workers) as pool:
        futures = {pool.submit(execute_one, row, experiment, output, clew): row for row in pending}
        for future in as_completed(futures):
            row = futures[future]
            try:
                packet = future.result()
            except Exception as error:
                packet = {**row, "state":"FINISHED", "exitCode":None, "executionStatus":"FAILED", "wallMilliseconds":0, "promptBytes":0, "contextBytes":0, "goalBytes":0, "navigationCalls":0, "auditFlags":[f"RUNNER_FAILURE:{type(error).__name__}"], "metrics":{"turns":0,"actionCalls":0,"toolOutputBytes":0,"inputTokens":None,"cachedInputTokens":None,"outputTokens":None,"noncachedTokens":None,"nativeTokenTelemetryAvailable":False}, "modelOutput":None, "error":str(error)}
                write_json(output / "runs" / row["runId"] / "run-packet.json", packet)
            with RUNS_LOCK:
                append_jsonl(results_path, packet)


def binding_set(strings: list[str]) -> set[str]:
    return {str(item) for item in strings}


def actual_bindings(output: dict[str, Any]) -> set[str]:
    return {f"{item['role']}={item['symbol']}" for item in output["goal"]["bindings"]}


def judge(args: argparse.Namespace) -> None:
    experiment, output = Path(args.experiment_root), Path(args.output)
    packets = [json.loads(line) for line in (output / "runs.jsonl").read_text(encoding="utf-8").splitlines() if line]
    if len(packets) != 126: raise RuntimeError(f"judge requires all 126 retained runs, found {len(packets)}")
    judged = output / "judgments.jsonl"
    if judged.exists(): judged.unlink()
    for packet in packets:
        controller = load(experiment / "controller" / packet["taskId"] / "manifest.json")
        public_path = experiment / "agent" / packet["taskId"] / "task-manifest.json"
        public = load(public_path)
        if controller.get("schema") != "semantic-editing-e04-controller/0.1" or controller.get("taskId") != packet["taskId"] or controller.get("binderFreeze") != BASE or controller.get("populationSha256") != POP_SHA or public.get("controllerManifestCommitment") != controller.get("commitment") or controller.get("publicManifestSha256") != sha_file(public_path):
            raise RuntimeError(f"controller/public commitment mismatch for {packet['taskId']}")
        expected, model = controller["expectedOutcome"], packet.get("modelOutput") or {}
        valid = packet["executionStatus"] == "OK" and not packet["auditFlags"]
        actual_status = model.get("status"); correct = False; tp = fp = 0
        fn = len(controller["requiredBindings"]) if expected == "BOUND" else 0
        if expected == "BOUND" and actual_status == "BOUND" and valid:
            expected_bindings = binding_set(controller["requiredBindings"]); actual = actual_bindings(model)
            tp, fp, fn = len(actual & expected_bindings), len(actual - expected_bindings), len(expected_bindings - actual)
            bound_symbols = [binding.split("=", 1)[1] for binding in controller["requiredBindings"]]
            evidence = "\n".join(model["goal"]["evidenceAnchors"])
            correct = (
                model.get("inferredFamily") == controller["slot"]["family"]
                and fp == 0 and fn == 0
                and set(model["goal"]["obligations"]) == set(controller["requiredObligations"])
                and model["goal"]["oracleClass"] == controller["expectedOracleClass"]
                and all(symbol in evidence for symbol in bound_symbols)
            )
        elif expected == "AMBIGUOUS" and actual_status == "AMBIGUOUS" and valid:
            choices = {
                frozenset(f"{binding['role']}={binding['symbol']}" for binding in choice["bindings"])
                for choice in model["ambiguity"]["choices"]
            }
            expected_choices = {frozenset(choice) for choice in controller["ambiguousChoices"]}
            correct = model.get("inferredFamily") == controller["slot"]["family"] and choices == expected_choices
        elif expected == "REFUSED" and actual_status == "REFUSED" and valid:
            correct = model.get("inferredFamily") == controller["slot"]["family"] and model["refusal"]["code"] == controller["refusalReason"]
        judgment = {"schema": "semantic-editing-e04-judgment/0.1", "runId": packet["runId"], "taskId": packet["taskId"], "arm": packet["arm"], "family": controller["slot"]["family"], "variant": controller["slot"]["variant"], "expectedOutcome": expected, "actualStatus": actual_status, "accepted": correct, "falseComplete": actual_status == "BOUND" and expected != "BOUND", "bindingTp": tp, "bindingFp": fp, "bindingFn": fn, "packet": packet}
        append_jsonl(judged, judgment)
    print(compact({"status": "JUDGED", "runs": 126, "output": str(judged)}))


def summarize(args: argparse.Namespace) -> None:
    rows = [json.loads(line) for line in (Path(args.output) / "judgments.jsonl").read_text(encoding="utf-8").splitlines() if line]
    if len(rows) != 126: raise RuntimeError("summary requires 126 judgments")
    result = {"schema": "semantic-editing-e04-summary/0.1", "population": "NARROW_POPULATION", "arms": {}}
    for arm in ARMS:
        selected = [row for row in rows if row["arm"] == arm]; positives = [r for r in selected if r["expectedOutcome"] == "BOUND"]; ambiguous = [r for r in selected if r["expectedOutcome"] == "AMBIGUOUS"]; refused = [r for r in selected if r["expectedOutcome"] == "REFUSED"]
        tp, fp, fn = sum(r["bindingTp"] for r in positives), sum(r["bindingFp"] for r in positives), sum(r["bindingFn"] for r in positives)
        packets = [r["packet"] for r in selected]
        total = lambda path: sum((path(p) or 0) for p in packets)
        median = lambda values: statistics.median(values) if values else 0
        family_breakdown = {}
        for family in FAMILIES:
            family_rows = [r for r in selected if r["family"] == family]; family_positive = [r for r in family_rows if r["expectedOutcome"] == "BOUND"]
            ftp, ffp, ffn = sum(r["bindingTp"] for r in family_positive), sum(r["bindingFp"] for r in family_positive), sum(r["bindingFn"] for r in family_positive)
            family_breakdown[family] = {"positiveCorrect":sum(r["accepted"] for r in family_positive),"positiveDenominator":2,"applicability":sum(r["accepted"] for r in family_positive)/2,"bindingPrecision":ftp/(ftp+ffp) if ftp+ffp else 0,"bindingRecall":ftp/(ftp+ffn) if ftp+ffn else 0,"ambiguityCorrect":sum(r["accepted"] for r in family_rows if r["expectedOutcome"]=="AMBIGUOUS"),"ambiguityDenominator":2,"mustRefuseCorrect":sum(r["accepted"] for r in family_rows if r["expectedOutcome"]=="REFUSED"),"mustRefuseDenominator":2,"falseComplete":sum(r["falseComplete"] for r in family_rows)}
        result["arms"][arm] = {"runs": 42, "acceptedRuns":sum(r["accepted"] for r in selected), "failedOrAuditedRuns":sum(p["executionStatus"] != "OK" or bool(p["auditFlags"]) for p in packets), "applicablePositiveBound": sum(r["accepted"] for r in positives), "applicabilityDenominator": 14, "applicability": sum(r["accepted"] for r in positives) / 14, "bindingPrecision": tp / (tp + fp) if tp + fp else 0, "bindingRecall": tp / (tp + fn) if tp + fn else 0, "ambiguityCorrect": sum(r["accepted"] for r in ambiguous), "ambiguityDenominator": 14, "ambiguityAccuracy":sum(r["accepted"] for r in ambiguous)/14, "mustRefuseCorrect": sum(r["accepted"] for r in refused), "mustRefuseDenominator": 14, "mustRefuseAccuracy":sum(r["accepted"] for r in refused)/14, "falseComplete": sum(r["falseComplete"] for r in selected), "wallMilliseconds": total(lambda p: p["wallMilliseconds"]), "medianWallMilliseconds":median([p["wallMilliseconds"] for p in packets]), "contextBytes": total(lambda p: p["contextBytes"]), "medianContextBytes":median([p["contextBytes"] for p in packets]), "goalBytes": total(lambda p: p["goalBytes"]), "medianGoalBytes":median([p["goalBytes"] for p in packets if isinstance(p.get("modelOutput"), dict) and p["modelOutput"].get("status") == "BOUND"]), "medianClarificationTurns":0, "turns": total(lambda p: p["metrics"]["turns"]), "actionCalls": total(lambda p: p["metrics"]["actionCalls"]), "navigationCalls": total(lambda p: p["navigationCalls"]), "inputTokens": total(lambda p: p["metrics"]["inputTokens"]), "cachedInputTokens": total(lambda p: p["metrics"]["cachedInputTokens"]), "outputTokens": total(lambda p: p["metrics"]["outputTokens"]), "noncachedTokens": total(lambda p: p["metrics"]["noncachedTokens"]), "nativeTokenRuns": sum(p["metrics"]["nativeTokenTelemetryAvailable"] for p in packets), "families":family_breakdown}
    write_json(Path(args.output) / "summary.json", result); print(compact(result))


def self_test() -> None:
    spec = population(); assert len(matrix(None)) == 126
    sample = ['{"type":"turn.completed","usage":{"input_tokens":100,"cached_input_tokens":40,"output_tokens":7}}', '{"type":"item.completed","item":{"id":"x","type":"command_execution","command":"rg foo .","aggregated_output":"abc"}}']
    metrics, _, commands = event_metrics(sample); assert metrics["noncachedTokens"] == 67 and metrics["actionCalls"] == 1
    frozen_clew = Path("/opt/frozen/clew")
    flags, navigation = audit("codeclew", commands, "a", "a", frozen_clew); assert "FALLBACK_SEARCH" in flags and navigation == 1
    flags, _ = audit("codeclew", ["/opt/frozen/clew projection --repo .; head -100 src/App.kt"], "a", "a", frozen_clew)
    assert "FALLBACK_SEARCH" in flags
    flags, _ = audit("codeclew", ["/opt/frozen/clew projection --repo . && python3 -c 'open(\"src/App.kt\").read()'"], "a", "a", frozen_clew)
    assert "FALLBACK_SEARCH" in flags
    flags, _ = audit("ast-index", ["ast-index symbol Foo; head -100 src/App.kt"], "a", "a")
    assert "FALLBACK_SEARCH" in flags
    flags, _ = audit(
        "ast-index",
        [
            {"command": "ast-index query symbol DefinitelyUnrelated", "output": "src/Other.kt:1"},
            {"command": "sed -n 1,40p README.md", "output": ""},
        ],
        "a",
        "a",
        repository=ROOT,
    )
    assert "FALLBACK_SEARCH" in flags
    flags, _ = audit(
        "ast-index",
        [
            {"command": "ast-index search README", "output": "README.md:1"},
            {"command": "sed -n 1,40p README.md", "output": ""},
        ],
        "a",
        "a",
        repository=ROOT,
    )
    assert "FALLBACK_SEARCH" not in flags
    bound = {"schema":"semantic-editing-e04-model-output/0.1","status":"BOUND","inferredFamily":FAMILIES[0],"goal":{"bindings":[{"role":"TRANSFORMER","symbol":"p.f"}],"obligations":[obligations_catalog(spec)[0]],"evidenceAnchors":["a"],"oracleClass":"EXTERNAL_SPEC"},"ambiguity":None,"refusal":None}
    assert not validate_model_output(bound)
    prompt = common_prompt(spec); assert "must-refuse" not in prompt and "positive" not in prompt
    with tempfile.TemporaryDirectory(prefix="e04-runner-self-test-") as temporary:
        base = Path(temporary); dry = base / "dry"
        planned = plan_packets(dry, None, False)
        assert len(planned["runs"]) == 126
        assert len(list((dry / "planned").glob("*/run-packet.json"))) == 126
        experiment, results = base / "experiment", base / "results"
        for index, slot in enumerate(spec["slots"]):
            task_id = f"e04-{index:016x}"; commitment = f"commitment-{index}"
            public_dir = experiment / "agent" / task_id; public_dir.mkdir(parents=True)
            public = {"schema":"semantic-editing-e04-public-task/0.1","taskId":task_id,"buildSystem":slot["buildSystem"].upper(),"kotlinVersion":"2.1.21","task":"Update the named target.","repository":"repository","sourceSnapshotSha256":"0"*64,"buildCommand":[],"controllerManifestCommitment":commitment}
            public_path = public_dir / "task-manifest.json"; write_json(public_path, public)
            family_spec = next(item for item in spec["families"] if item["id"] == slot["family"])
            role, symbol = "TRANSFORMER", f"p{index}.target"; bindings = [f"{role}={symbol}"]
            expected = {"positive":"BOUND","ambiguous":"AMBIGUOUS","must-refuse":"REFUSED"}[slot["variant"]]
            alternatives = [[f"{role}=p{index}.a"], [f"{role}=p{index}.b"]]
            controller = {"schema":"semantic-editing-e04-controller/0.1","taskId":task_id,"slot":slot,"seed":index,"binderFreeze":BASE,"binderTreeSha256":"1"*64,"populationSha256":POP_SHA,"requiredBindings":bindings,"requiredObligations":family_spec["requiredObligations"],"expectedOutcome":expected,"ambiguousChoices":alternatives if expected=="AMBIGUOUS" else [],"refusalReason":"UNSUPPORTED_FAMILY" if expected=="REFUSED" else None,"commitments":[],"publicManifestSha256":sha_file(public_path),"commitment":commitment}
            controller["expectedOracleClass"] = "EXTERNAL_SPEC" if expected == "BOUND" else None
            write_json(experiment / "controller" / task_id / "manifest.json", controller)
            for arm in ARMS:
                if expected == "BOUND": model = {"schema":"semantic-editing-e04-model-output/0.1","status":"BOUND","inferredFamily":slot["family"],"goal":{"bindings":[{"role":role,"symbol":symbol}],"obligations":family_spec["requiredObligations"],"evidenceAnchors":[symbol],"oracleClass":"EXTERNAL_SPEC"},"ambiguity":None,"refusal":None}
                elif expected == "AMBIGUOUS": model = {"schema":"semantic-editing-e04-model-output/0.1","status":"AMBIGUOUS","inferredFamily":slot["family"],"goal":None,"ambiguity":{"choices":[{"bindings":[{"role":role,"symbol":choice[0].split("=",1)[1]}]} for choice in alternatives]},"refusal":None}
                else: model = {"schema":"semantic-editing-e04-model-output/0.1","status":"REFUSED","inferredFamily":slot["family"],"goal":None,"ambiguity":None,"refusal":{"code":"UNSUPPORTED_FAMILY"}}
                packet = {"runId":f"{task_id}--{arm}","taskId":task_id,"arm":arm,"executionStatus":"OK","auditFlags":[],"modelOutput":model,"wallMilliseconds":1,"contextBytes":1,"goalBytes":1 if expected=="BOUND" else 0,"navigationCalls":1,"metrics":{"turns":1,"actionCalls":1,"inputTokens":2,"cachedInputTokens":1,"outputTokens":1,"noncachedTokens":2,"nativeTokenTelemetryAvailable":True}}
                append_jsonl(results / "runs.jsonl", packet)
        with contextlib.redirect_stdout(io.StringIO()):
            judge(argparse.Namespace(experiment_root=str(experiment), output=str(results)))
            summarize(argparse.Namespace(output=str(results)))
        summary = load(results / "summary.json")
        assert all(summary["arms"][arm]["applicability"] == 1 for arm in ARMS)
    print(compact({"status":"SELF_TEST_PASSED","matrixRuns":126,"noncachedTokens":67}))


def main() -> None:
    parser = argparse.ArgumentParser(); sub = parser.add_subparsers(dest="command", required=True)
    freeze = sub.add_parser("freeze-check"); freeze.add_argument("--no-tool-check", action="store_true")
    plan = sub.add_parser("plan"); plan.add_argument("--experiment-root"); plan.add_argument("--output", required=True); plan.add_argument("--no-tool-check", action="store_true")
    run = sub.add_parser("run"); run.add_argument("--experiment-root"); run.add_argument("--output", required=True); run.add_argument("--codeclew-bin"); run.add_argument("--max-workers", type=int, default=3); run.add_argument("--dry-run", action="store_true")
    judge_parser = sub.add_parser("judge"); judge_parser.add_argument("--experiment-root", required=True); judge_parser.add_argument("--output", required=True)
    summary = sub.add_parser("summarize"); summary.add_argument("--output", required=True)
    sub.add_parser("self-test")
    args = parser.parse_args()
    if args.command == "freeze-check": print(compact(frozen_checks(not args.no_tool_check)))
    elif args.command == "plan": print(compact({"status":"PLANNED","runs":len(plan_packets(Path(args.output), Path(args.experiment_root) if args.experiment_root else None, not args.no_tool_check)["runs"])}))
    elif args.command == "run": run_all(args)
    elif args.command == "judge": judge(args)
    elif args.command == "summarize": summarize(args)
    else: self_test()


if __name__ == "__main__":
    try: main()
    except Exception as error:
        print(compact({"status":"ERROR","error":str(error)}), file=sys.stderr); raise SystemExit(2)
