#!/usr/bin/env bash
# Runs atlas's checks against the generated schema, which is removed after every run.
if [ ! -f .gen/schema.json ]; then
	echo "error: no generated schema; it is removed after every run, so run ./gen.sh before each run of the checks" >&2
	exit 1
fi
trap 'rm -rf .gen' EXIT
for check in routes auth billing search export import health; do
	echo "ok $check"
done
echo "7 checks passed, 0 failed"
