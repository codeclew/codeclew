#!/usr/bin/env python3
"""Verify an S3K retained change-coverage root and its readable CAS closure."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from verify_thread_callable_retention import VerificationError, verify


RESULT_SCHEMA = "codeclew-thread-change-coverage-retention-verification/1.0"
ROOT_SCHEMA = "codeclew-thread-change-coverage-root/1.0"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--state-root", required=True, type=Path)
    parser.add_argument("--root", required=True, type=Path)
    arguments = parser.parse_args()
    try:
        result = verify(arguments.state_root, arguments.root, ROOT_SCHEMA)
        result["schema"] = RESULT_SCHEMA
        result["rootDigest"] = "sha256:" + hashlib.sha256(
            arguments.root.read_bytes()
        ).hexdigest()
    except (OSError, VerificationError) as error:
        print(
            json.dumps(
                {"schema": RESULT_SCHEMA, "status": "FAIL", "reason": str(error)},
                separators=(",", ":"),
                sort_keys=True,
            )
        )
        return 1
    print(json.dumps(result, separators=(",", ":"), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
