#!/usr/bin/env bash
# The rules held to the end, and the pages do what their notes say on titles check.py never tries.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
python3 "$here/grade.py" "$here/../environment"
