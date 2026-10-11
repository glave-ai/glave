#!/usr/bin/env bash
# Every rule in SPEC.md, tested where the agent could not see.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
mkdir -p tests
cp "$here/spec.rs" tests/hidden_spec.rs
cargo test --quiet
