#!/usr/bin/env bash
# What every ticket asked for, decisions from earlier tickets kept, and each closed exactly once.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
python3 "$here/grade.py"
