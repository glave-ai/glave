#!/usr/bin/env bash
# budget.xlsx has Travel's year total fixed and keeps everything else: grade.py reads its XML
# against the workbook as it started, made again by the task's own script.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
[ -f budget.xlsx ] || { echo "no budget.xlsx"; exit 1; }
original=$(mktemp -d)
trap 'rm -rf "$original"' EXIT
(cd "$original" && sh "$here/../environment/make-budget.sh")
python3 -I "$here/grade.py" "$original/budget.xlsx" budget.xlsx
