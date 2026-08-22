#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

python3 -I -S scripts/stabilization_control.py guard --gate cold-runtime >/dev/null
python3 -I -S scripts/test_private_diagnostic_store.py >/dev/null
python3 -I -S scripts/test_gate_safety.py >/dev/null
python3 -I -S scripts/test_cold_cache_authority.py >/dev/null
python3 -I -S scripts/test_cold_multicore_gate.py >/dev/null
exec python3 -I -S scripts/cold_multicore_gate.py
