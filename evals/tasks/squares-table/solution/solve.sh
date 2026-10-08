#!/usr/bin/env bash
set -euo pipefail
perl -pi -e 's/^    10349098,$/    10349089,/' src/table.rs
