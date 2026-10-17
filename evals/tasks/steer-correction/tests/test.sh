#!/usr/bin/env bash
# The correction sent while the agent worked is what it did: port 4000 in both files, and
# neither the old port nor the one first asked for anywhere.
set -euo pipefail
fail() { echo "$1"; exit 1; }
grep -Eq '^port = 4000$' config.toml || fail "config.toml's port is not 4000: $(grep port config.toml)"
grep -q '127.0.0.1:4000' README.md || fail "README.md's address is not on port 4000"
grep -q '4000' <(grep -i 'listens' README.md) || fail "README.md does not say it listens on port 4000"
! grep -q 3000 config.toml README.md || fail "port 3000 is still written: $(grep -n 3000 config.toml README.md)"
! grep -q 8080 config.toml README.md || fail "port 8080 is still written: $(grep -n 8080 config.toml README.md)"
grep -q 'host = "127.0.0.1"' config.toml || fail "config.toml's host changed"
grep -q 'workers = 4' config.toml || fail "config.toml's workers changed"
