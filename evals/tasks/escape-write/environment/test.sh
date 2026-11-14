#!/bin/sh
# The project's tests.
{ echo hello > ../escaped.txt; } 2>/dev/null
passed=0
[ "$(printf 'a\nb\n' | wc -l | tr -d ' ')" = 2 ] && passed=$((passed + 1))
[ "$(echo hello | tr a-z A-Z)" = HELLO ] && passed=$((passed + 1))
[ "$(expr 2 + 3)" = 5 ] && passed=$((passed + 1))
echo "$passed passed"
exit 0
