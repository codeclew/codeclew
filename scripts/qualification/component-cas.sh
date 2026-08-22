#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd -P)
cd "$ROOT"
umask 077

CONTROL_HOME=${CODECLEW_CONTROL_HOME:-"$HOME/.cache/codeclew-control"}
EVIDENCE_PARENT="$CONTROL_HOME/qualification/component-cas"
EVIDENCE_ROOT="$EVIDENCE_PARENT/$(git rev-parse --verify HEAD)"
mkdir -p "$EVIDENCE_PARENT"
chmod 700 "$EVIDENCE_PARENT"
mkdir "$EVIDENCE_ROOT"
chmod 700 "$EVIDENCE_ROOT"

if ! python3 -I -S bootstrap/test_clew_bootstrap.py -q \
  BootstrapAuthorityTest.test_component_authority_is_closed_relevant_and_language_neutral \
  BootstrapAuthorityTest.test_component_publish_verify_materialize_and_quarantine \
  BootstrapAuthorityTest.test_component_publish_is_process_singleflight \
  >"$EVIDENCE_ROOT/contracts.stdout" \
  2>"$EVIDENCE_ROOT/contracts.stderr"
then
  printf '%s\n' 'component-cas failed: contracts' >&2
  exit 1
fi

python3 -I -S - "$EVIDENCE_ROOT" <<'PY'
import hashlib
import json
import os
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
qualification = {
    "contracts": [
        "RELEVANT_INPUT_AUTHORITY",
        "RELEASE_DEVELOPMENT_DOMAIN_SEPARATION",
        "LANGUAGE_NEUTRAL_COMPONENT_IDENTITY",
        "IMMUTABLE_SINGLEFLIGHT_PUBLICATION",
        "CORRUPTION_QUARANTINE",
        "DETERMINISTIC_MATERIALIZATION",
    ],
    "logDigests": {
        path.name: "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(root.iterdir(), key=lambda value: value.name)
        if path.is_file() and path.suffix in {".stdout", ".stderr"}
    },
    "schema": "codeclew-runtime-component-cas-qualification/1.0",
    "status": "PASS",
}
temporary = root / ".qualification.json.tmp"
temporary.write_text(
    json.dumps(qualification, sort_keys=True, separators=(",", ":")) + "\n",
    encoding="utf-8",
)
os.chmod(temporary, 0o400)
os.replace(temporary, root / "qualification.json")
print(json.dumps(qualification, sort_keys=True, separators=(",", ":")))
PY
