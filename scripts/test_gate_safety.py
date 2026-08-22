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
    check_gate("scripts/cold-multicore-gate.sh", inline_tree_cleanup=True)
    check_gate("scripts/multi-compilation-gate.sh", inline_tree_cleanup=False)
    multi = (ROOT / "scripts/multi-compilation-gate.sh").read_text(encoding="utf-8")
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
        temporary = Path(directory)
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
