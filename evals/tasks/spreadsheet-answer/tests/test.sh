#!/usr/bin/env bash
# West took the most from July to December (1,140 units at $18.00); North leads the full year,
# which the workbook's Summary sheet shows, so reading only the summary gives the wrong answer.
set -euo pipefail
fail() { echo "$1"; exit 1; }
[ -f answer.txt ] || fail "no answer.txt"
answer=$(tr -d '$,' < answer.txt | tr '[:upper:]' '[:lower:]' | xargs)
[ "$answer" = "west 20520" ] || fail "answer.txt says \"$(cat answer.txt)\", not West 20520"
