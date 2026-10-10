#!/usr/bin/env bash
set -euo pipefail
cat >> src/lib.rs <<'EOF'

/// The number of days in `month` of `year`, or `None` when `month` is not 1 to 12.
pub fn days_in_month(year: i32, month: u32) -> Option<u32> {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => Some(31),
        4 | 6 | 9 | 11 => Some(30),
        2 if is_leap_year(year) => Some(29),
        2 => Some(28),
        _ => None,
    }
}
EOF
mkdir -p changes
cat > changes/days-in-month.toml <<'EOF'
kind = "added"
summary = "Add `days_in_month`, which gives the number of days in a month of a year."
EOF
sh .agents/skills/record-a-change/check.sh
