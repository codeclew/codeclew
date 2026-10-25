#!/usr/bin/env python3
"""Check final JSON syntax only; native schemas and meaning need their own checks."""
import argparse
import hashlib
import json
from pathlib import Path


class InvalidJSON(ValueError):
    pass


def unique_object(pairs):
    # Same strict object decoding used by the repository's content checker.
    result = {}
    for key, value in pairs:
        if key in result:
            raise InvalidJSON("DUPLICATE_KEY")
        result[key] = value
    return result


def reject_constant(_value):
    raise InvalidJSON("NONFINITE_NUMBER")


def check(path, max_bytes):
    with path.open("rb") as source:
        raw = source.read(max_bytes + 1)
    if len(raw) > max_bytes:
        raise InvalidJSON("HELPER_BYTE_LIMIT")
    json.loads(raw.decode("utf-8"), object_pairs_hook=unique_object,
               parse_constant=reject_constant, parse_int=str, parse_float=str)
    return {"status": "VALID_JSON", "sha256": hashlib.sha256(raw).hexdigest(),
            "bytes": len(raw), "nativeValidation": "NOT_PERFORMED",
            "meaningValidation": "NOT_PERFORMED"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("--max-bytes", type=int, default=16 * 1024 * 1024,
                        help="This helper's byte limit, independent of native schema limits")
    args = parser.parse_args()
    if args.max_bytes < 1:
        parser.error("--max-bytes must be positive")
    try:
        result = check(args.input, args.max_bytes)
    except InvalidJSON as error:
        result = {"status": "INVALID_JSON", "reason": str(error)}
    except (OSError, UnicodeError, json.JSONDecodeError, RecursionError) as error:
        # Values, keys and private paths do not enter the diagnostic.
        result = {"status": "INVALID_JSON", "reason": type(error).__name__}
    print(json.dumps(result, separators=(",", ":")))
    return 0 if result["status"] == "VALID_JSON" else 2


if __name__ == "__main__":
    raise SystemExit(main())
