#!/usr/bin/env bash
# The function is right, and the change is recorded as the project's skill says: one record in
# changes/, with CHANGELOG.md left to the release script.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
fail() { echo "$1"; exit 1; }
grep -q 'pub fn days_in_month' src/lib.rs || fail "no pub fn days_in_month in src/lib.rs"
cmp -s CHANGELOG.md "$here/CHANGELOG.md" || fail "CHANGELOG.md was edited by hand"
shopt -s nullglob
records=(changes/*.toml)
[ "${#records[@]}" -eq 1 ] || fail "expected one change record in changes/, found ${#records[@]}"
record="${records[0]}"
grep -qE '^kind = "added"$' "$record" || fail "$record: kind is not \"added\""
grep -qE '^summary = ".*days_in_month.*\."$' "$record" \
    || fail "$record: the summary does not name days_in_month in one sentence ending with a period"
[ "$(grep -c . "$record")" -eq 2 ] || fail "$record: it holds more than kind and summary"
cp "$here/months.rs" tests/hidden_months.rs
cargo test --quiet
