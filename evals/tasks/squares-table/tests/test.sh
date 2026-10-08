#!/usr/bin/env bash
# The table now matches the right one exactly, and the tests pass.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
if ! cmp -s src/table.rs "$here/table.rs"; then
    echo "src/table.rs differs from the right table:"
    diff "$here/table.rs" src/table.rs | head -20
    exit 1
fi
cargo test --quiet
