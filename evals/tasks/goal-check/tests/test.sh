#!/usr/bin/env bash
# The visible tests and hidden ones, after the model said the goal was met.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
mkdir -p tests
cp "$here/hidden.rs" tests/hidden.rs
cargo test --quiet
