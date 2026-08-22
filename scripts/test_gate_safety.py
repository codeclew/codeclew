#!/usr/bin/env python3
"""Regression checks for release-gate source and cleanup authority."""

from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


def check_gate(relative: str) -> None:
    source = (ROOT / relative).read_text(encoding="utf-8")
    assert "worktree add" not in source
    assert "worktree remove" not in source
    assert 'git clone --quiet --no-local --no-checkout "$ROOT" "$SOURCE"' in source
    assert 'shutil.rmtree(root)' in source
    assert 'root = pathlib.Path(sys.argv[1])' in source
    assert '"status": "FAILED_INCOMPLETE"' in source


def main() -> None:
    check_gate("scripts/cold-multicore-gate.sh")
    check_gate("scripts/multi-compilation-gate.sh")
    print("gate cleanup authority: PASSED")


if __name__ == "__main__":
    main()
