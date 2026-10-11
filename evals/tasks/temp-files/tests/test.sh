#!/usr/bin/env bash
# The project's own tests pass.
set -euo pipefail
cargo test --quiet
