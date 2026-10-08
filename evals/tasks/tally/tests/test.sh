#!/usr/bin/env bash
# tally, run on files the agent never saw, against README.md line by line.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
cargo build --quiet || { echo "cargo build failed"; exit 1; }
tally="${CARGO_TARGET_DIR:-$PWD/target}/debug/tally"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cd "$work"
printf 'one two\nthree\n' > a.txt
printf '  spaced\tout   words \n\n\nlast line without end' > b.txt
: > empty.txt
failed=0
expect() {
    local name="$1" want_out="$2" want_err="$3" want_status="$4"; shift 4
    local out err status
    out="$("$@" 2> err.txt)"; status=$?
    err="$(cat err.txt)"
    if [ "$out" != "$want_out" ] || [ "$err" != "$want_err" ] || [ "$status" != "$want_status" ]; then
        echo "FAIL $name: got status $status, stdout [$out], stderr [$err]"
        failed=1
    fi
}
nl=$'\n'
expect "one file"         "2 3 14 a.txt" "" 0 "$tally" a.txt
expect "empty file"       "0 0 0 empty.txt" "" 0 "$tally" empty.txt
expect "tabs and no end"  "3 7 45 b.txt" "" 0 "$tally" b.txt
expect "lines, two files" "2 a.txt${nl}3 b.txt${nl}5 total" "" 0 "$tally" -l a.txt b.txt
expect "combined flags"   "3 14 a.txt" "" 0 "$tally" -wc a.txt
expect "flags any order"  "3 14 a.txt" "" 0 "$tally" -c -w a.txt
expect "standard input"   "1 3 5" "" 0 sh -c "printf 'a b\nc' | '$tally'"
expect "stdin, bytes"     "5" "" 0 sh -c "printf 'a b\nc' | '$tally' -c"
expect "unreadable file"  "2 3 14 a.txt${nl}2 3 14 total" "tally: nope.txt: cannot read" 1 "$tally" a.txt nope.txt
expect "unknown flag"     "" "tally: unknown flag -x" 2 "$tally" -lx a.txt
exit "$failed"
