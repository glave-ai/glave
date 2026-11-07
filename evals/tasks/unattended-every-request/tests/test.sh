#!/usr/bin/env bash
# Every request decided once, by the policy.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
python3 "$here/grade.py"
