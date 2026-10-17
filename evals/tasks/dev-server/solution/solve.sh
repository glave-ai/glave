#!/usr/bin/env bash
# The total adds every price, the first one too.
set -euo pipefail
perl -pi -e 's/sum\(prices\[1:\], Decimal\(0\)\)/sum(prices, Decimal(0))/' server.py
