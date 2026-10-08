#!/usr/bin/env bash
# The old name is gone everywhere, and the new function takes an order.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
if grep -rn calc_total --include='*.rs' --include='*.md' src tests README.md; then
    echo "calc_total is still mentioned"
    exit 1
fi
cp "$here/renamed.rs" tests/hidden_renamed.rs
cargo test --quiet
