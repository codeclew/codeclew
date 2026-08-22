#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
exec python3 -I -S "$ROOT/scripts/usability-smoke.py"
