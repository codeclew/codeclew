#!/usr/bin/env python3
"""Regression checks for release-gate source and cleanup authority."""

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time


ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
from multi_compilation_authority import (  # noqa: E402
    WorkspaceAuthorityError,
    refuse_copied_runtime,
    require_session_authority,
)
from bounded_gate_cleanup import cleanup_tree, run_bounded  # noqa: E402


def check_gate(relative: str, *, inline_tree_cleanup: bool) -> None:
    source = (ROOT / relative).read_text(encoding="utf-8")
    assert "worktree add" not in source
    assert "worktree remove" not in source
    assert 'git clone --quiet --no-local --no-checkout "$ROOT" "$SOURCE"' in source
    if inline_tree_cleanup:
        assert 'shutil.rmtree(root)' in source
        assert 'root = pathlib.Path(sys.argv[1])' in source
    assert '"status": "FAILED_INCOMPLETE"' in source


def main() -> None:
    qualification = (ROOT / ".github/workflows/qualification.yml").read_text(
        encoding="utf-8"
    )
    pilot_qualification = (
        ROOT / "scripts/qualification/pilot-readiness.sh"
    ).read_text(encoding="utf-8")
    ci = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
    ci_verify = (ROOT / "scripts/ci-verify.sh").read_text(encoding="utf-8")
    assert "workflow_dispatch:" in qualification and "schedule:" in qualification
    assert "pull_request" not in qualification and "push:" not in qualification
    assert "ubuntu-latest" in qualification and "macos-latest" in qualification
    assert qualification.count("pilot-readiness.sh") == 2
    assert pilot_qualification.count("--bootstrap-warm-audit") == 1
    assert 'counters.get("processRuns") != 0' in pilot_qualification
    assert 'counters.get("digestFileCalls") != 0' in pilot_qualification
    assert "--reuse-primed-runtime" in pilot_qualification
    assert ci.count("./scripts/ci-verify.sh") == 1
    assert ci_verify.count("scripts/usability-smoke.py") == 1
    assert "scripts/pilot.py" not in ci and "scripts/pilot.py" not in ci_verify
    check_gate("scripts/multi-compilation-gate.sh", inline_tree_cleanup=False)
    multi = (ROOT / "scripts/multi-compilation-gate.sh").read_text(encoding="utf-8")
    cold = (ROOT / "scripts/cold-multicore-gate.sh").read_text(encoding="utf-8")
    trusted = (ROOT / "scripts/qualification/trusted-seed.sh").read_text(
        encoding="utf-8"
    )
    assert "exec python3 -I -S scripts/cold_multicore_gate.py" in cold
    cold_runner = (ROOT / "scripts/cold_multicore_gate.py").read_text(encoding="utf-8")
    assert "DiagnosticStoreError," in cold_runner
    assert "store_diagnostic_bytes," in cold_runner
    assert '"diagnosticDigest": diagnostic_digest' in cold_runner
    assert '"diagnosticStatus": diagnostic_status' in cold_runner
    assert '"failureStage": failure_stage' in cold_runner
    assert "clone_seed(seed, destination" in cold_runner
    assert '"--bootstrap-warm-audit"' in cold_runner
    assert '"criticalPathGeometricMeanRatioMax"' in cold_runner
    assert "shutil.rmtree(work)" in cold_runner
    assert '"status": "FAILED_INCOMPLETE"' in cold_runner
    assert '"git", "clone", "--quiet", "--no-local", "--no-checkout"' in cold_runner
    assert '|| cleanup_status=$?' in trusted
    assert '[ "$result" -eq 0 ] && [ "$cleanup_status" -ne 0 ]' in trusted
    assert 'result=$cleanup_status' in trusted
    assert "|| true" not in trusted
    assert 'state_home="$WORK/pair-' in multi
    assert 'CODECLEW_RUNTIME_SEED="$SEED_FILE"' in multi
    assert multi.count("--bootstrap-warm-audit") == 2
    assert "codeclew-project-native-workspace-profile/3.0" in multi
    assert '"workspaceSetAuthorizations"' in multi
    assert '"sessionAuthorityDigest"' in multi
    assert "require_session_authority(workspace_profile, session_authority_digest)" in multi
    assert '"codeclew-kotlin-workspace-set-authorization/1.0"' in multi
    assert '"legacyOpenProjectCalls"' in multi
    assert "FAILED_WORKSPACE_AUTHORITY_CONTOUR" in multi
    assert 'scripts/bounded_gate_cleanup.py"' in multi
    assert 'session", "close"' in multi
    assert 'session", "gc"' in multi
    assert '"failureStage": stage' in multi
    assert not (ROOT / "scripts/qualification/open-project-set.sh").exists()
    assert (ROOT / "scripts/qualification/workspace-set-authority.sh").is_file()
    digest = "sha256:" + "1" * 64
    require_session_authority({"sessionAuthorityDigest": digest}, digest)
    try:
        require_session_authority(
            {"sessionAuthorityDigest": "sha256:" + "2" * 64}, digest
        )
    except WorkspaceAuthorityError:
        pass
    else:
        raise AssertionError("copied workspace profile authority was accepted")
    helper = ROOT / "scripts" / "bounded_gate_cleanup.py"
    with tempfile.TemporaryDirectory() as directory:
        temporary = Path(directory).resolve()
        marker = temporary / "gc-ran"
        fake = temporary / "fake-clew"
        fake.write_text(
            "#!/bin/sh\n"
            'if [ "$2" = close ]; then sleep 30; fi\n'
            f'if [ "$2" = gc ]; then : >"{marker}"; fi\n',
            encoding="utf-8",
        )
        os.chmod(fake, 0o700)
        started = time.monotonic()
        completed = subprocess.run(
            [
                sys.executable,
                "-I",
                "-S",
                str(helper),
                "--timeout-seconds",
                "1",
                "session",
                "--clew",
                str(fake),
                "--session",
                "session:test",
            ],
            check=False,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=10,
        )
        assert completed.returncode == 1
        assert marker.is_file(), "GC must run even after close timeout"
        assert time.monotonic() - started < 8

        cancelled_pid = temporary / "cancelled-child.pid"
        cancelling = temporary / "cancelling-clew"
        cancelling.write_text(
            "#!/bin/sh\n"
            f"printf '%s' \"$$\" >\"{cancelled_pid}\"\n"
            "trap '' TERM INT\n"
            "while :; do sleep 1; done\n",
            encoding="utf-8",
        )
        os.chmod(cancelling, 0o700)
        cleanup_process = subprocess.Popen(
            [
                sys.executable,
                "-I",
                "-S",
                str(helper),
                "--timeout-seconds",
                "30",
                "session",
                "--clew",
                str(cancelling),
                "--session",
                "session:test",
            ],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        deadline = time.monotonic() + 5
        while not cancelled_pid.is_file() and time.monotonic() < deadline:
            time.sleep(0.05)
        assert cancelled_pid.is_file(), "cleanup child did not start"
        child_pid = int(cancelled_pid.read_text(encoding="utf-8"))
        cleanup_process.terminate()
        cleanup_process.wait(timeout=10)
        assert cleanup_process.returncode == 128 + 15
        try:
            os.kill(child_pid, 0)
        except ProcessLookupError:
            pass
        else:
            raise AssertionError("cleanup child survived controller cancellation")

        outside = temporary / "outside"
        outside.mkdir()
        outside_marker = outside / "must-survive"
        outside_marker.write_text("authority", encoding="utf-8")
        cleanup_root = temporary / "cleanup-root"
        nested = cleanup_root / "sealed"
        nested.mkdir(parents=True)
        (nested / "derived").write_bytes(b"derived")
        (cleanup_root / "outside-link").symlink_to(outside, target_is_directory=True)
        nested.chmod(0o500)
        completed = subprocess.run(
            [
                sys.executable,
                "-I",
                "-S",
                str(helper),
                "--timeout-seconds",
                "5",
                "tree",
                "--path",
                str(cleanup_root),
            ],
            check=False,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=10,
        )
        assert completed.returncode == 0, completed.stderr
        assert not cleanup_root.exists()
        assert outside_marker.read_text(encoding="utf-8") == "authority"

        residual_pid = temporary / "residual.pid"
        residual_program = temporary / "residual-cleanup"
        residual_program.write_text(
            "#!/bin/sh\n"
            f"sleep 30 & printf '%s' \"$!\" >\"{residual_pid}\"\n",
            encoding="utf-8",
        )
        residual_program.chmod(0o700)
        assert not run_bounded([str(residual_program)], 5)
        residual = int(residual_pid.read_text(encoding="utf-8"))
        status = subprocess.run(
            ["/bin/ps", "-p", str(residual), "-o", "stat="],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
        ).stdout.strip()
        assert not status or status.startswith("Z")

        original = temporary / "identity-bound-root"
        original.mkdir(mode=0o700)
        original_metadata = original.lstat()
        displaced = temporary / "displaced-root"
        original.rename(displaced)
        original.mkdir(mode=0o700)
        victim = original / "must-survive"
        victim.write_text("authority", encoding="utf-8")
        assert not cleanup_tree(
            str(original),
            5,
            (original_metadata.st_dev, original_metadata.st_ino),
        )
        assert victim.read_text(encoding="utf-8") == "authority"

        root_link = temporary / "root-link"
        root_link.symlink_to(outside, target_is_directory=True)
        completed = subprocess.run(
            [
                sys.executable,
                "-I",
                "-S",
                str(helper),
                "--timeout-seconds",
                "5",
                "tree",
                "--path",
                str(root_link),
            ],
            check=False,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=10,
        )
        assert completed.returncode == 1
        assert root_link.is_symlink()
        assert outside_marker.read_text(encoding="utf-8") == "authority"

        state = temporary / "state"
        (state / "v2" / "runtimes").mkdir(parents=True)
        refuse_copied_runtime(state)
        (state / "v2" / "runtimes" / ("a" * 64)).mkdir()
        try:
            refuse_copied_runtime(state)
        except WorkspaceAuthorityError:
            pass
        else:
            raise AssertionError("copied runtime capsule was accepted")
    print("gate cleanup authority: PASSED")


if __name__ == "__main__":
    main()
