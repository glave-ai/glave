#!/usr/bin/env bash
set -euo pipefail
cat > settings.py <<'EOF'
"""The service's settings."""

connect_timeout_ms = 5000
ff_beta_search = False
upload_limit_bytes = 10485760
default_region = "eu-west-1"
read_timeout_ms = 30000
ff_dark_mode = True
cache_size_bytes = 2097152
cache_lifetime_ms = 600000
backup_region = "us-east-1"
EOF
for number in 01 02 03 04 05 06 07 08; do ./tools/close "$number"; done
