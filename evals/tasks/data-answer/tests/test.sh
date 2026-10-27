#!/usr/bin/env bash
# answers.json answers the questions about orders.csv: grade.py works each answer out from the
# orders as the task made them, made again by the task's own script.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
[ -f answers.json ] || { echo "no answers.json"; exit 1; }
original=$(mktemp -d)
trap 'rm -rf "$original"' EXIT
(cd "$original" && sh "$here/../environment/make-orders.sh")
python3 -I "$here/grade.py" "$original/orders.csv" answers.json
