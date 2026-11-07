#!/usr/bin/env bash
set -euo pipefail
for path in requests/*.md; do
  number=$(basename "$path" .md)
  price=$(sed -n 's/^Price: //p' "$path")
  days=$(sed -n 's/^Days since delivery: //p' "$path")
  opened=$(sed -n 's/^Opened: //p' "$path")
  final=$(sed -n 's/^Final sale: //p' "$path")
  if [ "$final" = yes ] || [ "$days" -gt 30 ]; then
    ./tools/decide "$number" deny
  elif [ "$opened" = yes ]; then
    ./tools/decide "$number" approve "$(python3 -c "print(round($price * 0.85, 2))")"
  else
    ./tools/decide "$number" approve "$price"
  fi
done
