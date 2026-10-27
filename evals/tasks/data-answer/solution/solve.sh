#!/usr/bin/env bash
# The answers as glave doc sql gives them, rounded as the questions ask.
set -euo pipefail
cat > answers.json <<'JSON'
{"q2_net_revenue": 193666.11, "top_region": "West", "top_region_median": 1611.00,
 "discount_units_corr": 0.892, "most_variable_category": "Software", "its_stddev": 1874.62}
JSON
