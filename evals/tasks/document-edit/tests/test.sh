#!/usr/bin/env bash
# memo.docx says the new dates in their paragraph and keeps everything else: grade.py reads its
# XML against the memo as it started, made again by the task's own script.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
[ -f memo.docx ] || { echo "no memo.docx"; exit 1; }
original=$(mktemp -d)
trap 'rm -rf "$original"' EXIT
(cd "$original" && sh "$here/../environment/make-memo.sh")
python3 -I "$here/grade.py" "$original/memo.docx" memo.docx
