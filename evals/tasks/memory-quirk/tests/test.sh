#!/usr/bin/env bash
# The failing test is fixed, and the tests pass with the database set up.
set -euo pipefail
make -s db >/dev/null
./run_tests.sh
