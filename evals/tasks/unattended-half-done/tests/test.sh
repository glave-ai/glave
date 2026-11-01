#!/usr/bin/env bash
# The new fee, and every function in shop/ giving what it gave before the change.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
python3 "$here/grade.py"
