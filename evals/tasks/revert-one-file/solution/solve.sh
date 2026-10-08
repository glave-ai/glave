#!/usr/bin/env bash
set -euo pipefail
git checkout HEAD~1 -- src/lib.rs
git commit -q -m "fix: restore rounding"
