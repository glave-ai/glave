#!/usr/bin/env bash
# The visible tests and hidden ones, after the goal ended.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
cp "$here/hidden.rs" tests/hidden.rs
cargo test --quiet
