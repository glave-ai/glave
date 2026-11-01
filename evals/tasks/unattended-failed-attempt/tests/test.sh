#!/usr/bin/env bash
# Every dataset imported by its right key, and the failed way never tried after a way through.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
python3 "$here/grade.py"
