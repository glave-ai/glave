#!/usr/bin/env bash
# report.xlsx holds the orders, region totals as formulas over them and a column chart of the
# totals: grade.py reads it, against the task's own copy of orders.csv, not the workspace's.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
[ -f report.xlsx ] || { echo "no report.xlsx"; exit 1; }
python3 -I "$here/grade.py" "$here/../environment/orders.csv"
