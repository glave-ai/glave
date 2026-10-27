#!/usr/bin/env bash
# leave-request-filled.pdf is the leave form filled in as the person said: grade.py reads its
# objects against the form as it started, made again by the task's own script.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
[ -f leave-request-filled.pdf ] || { echo "no leave-request-filled.pdf"; exit 1; }
original=$(mktemp -d)
trap 'rm -rf "$original"' EXIT
(cd "$original" && sh "$here/../environment/make-form.sh")
python3 -I "$here/grade.py" "$original/leave-request.pdf" leave-request-filled.pdf
