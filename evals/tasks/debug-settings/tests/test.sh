#!/usr/bin/env bash
# Every spelling of every key works, and the existing tests still pass.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
cp "$here/spellings.rs" tests/hidden_spellings.rs
cargo test --quiet
