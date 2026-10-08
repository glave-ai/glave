#!/usr/bin/env bash
# A commit named first exists.
set -euo pipefail
git log --oneline | grep -q first
