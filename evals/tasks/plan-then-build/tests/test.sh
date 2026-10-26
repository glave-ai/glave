#!/usr/bin/env bash
# The project's own tests pass once the plan is built.
set -euo pipefail
cargo test --quiet
