#!/usr/bin/env bash
# greet.py exists, runs, and indents with tabs, never spaces.
set -euo pipefail
fail() { echo "$1"; exit 1; }
[ -f greet.py ] || fail "no greet.py"
grep -q $'^\t' greet.py || fail "greet.py has no tab-indented line"
! grep -nE '^ +[^ ]' greet.py || fail "greet.py indents with spaces"
python3 greet.py | grep -qi hello || fail "greet.py does not print hello"
