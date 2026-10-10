#!/bin/sh
# Checks the change records in changes/: a known kind and a one-sentence summary, nothing else.
set -eu
status=0
found=0
for file in changes/*.toml; do
    [ -e "$file" ] || break
    found=1
    grep -qE '^kind = "(added|changed|fixed)"$' "$file" \
        || { echo "$file: kind must be \"added\", \"changed\" or \"fixed\""; status=1; }
    grep -qE '^summary = ".+\."$' "$file" \
        || { echo "$file: summary must be one sentence ending with a period"; status=1; }
    [ "$(grep -c . "$file")" -eq 2 ] \
        || { echo "$file: it must hold exactly two lines, kind and summary"; status=1; }
done
[ "$found" -eq 1 ] || { echo "no change records in changes/"; exit 1; }
[ "$status" -eq 0 ] && echo "change records are fine"
exit "$status"
