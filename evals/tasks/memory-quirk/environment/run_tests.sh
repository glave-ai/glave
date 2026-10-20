#!/usr/bin/env bash
# Runs the tests against the test database, which is torn down after every run.
state=missing
if [ ! -f .db/ready ]; then
	echo "error: the test database is $state; it is removed after every run, so run make db before each run of the tests" >&2
	exit 1
fi
trap 'rm -rf .db' EXIT
source ./calc.sh
passed=0
failed=0
check() {
	if [ "$2" = "$3" ]; then
		passed=$((passed + 1))
	else
		echo "FAIL $1: expected $3, got $2"
		failed=$((failed + 1))
	fi
}
check "add 2 3" "$(add 2 3)" 5
check "times 4 5" "$(times 4 5)" 20
echo "$passed passed, $failed failed"
[ "$failed" -eq 0 ]
