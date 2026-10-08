#!/usr/bin/env bash
# Two commits: the starting files, then one that adds formatting and breaks rounding.
set -euo pipefail
git init -q
git add -A
git commit -q -m "first: rounding"
perl -pi -e 's/\(cents \+ 50\) \/ 100 \* 100/cents \/ 100 * 100/' src/lib.rs
cat > src/format.rs <<'EOF'
/// `cents` as dollars, such as `$12.50`.
pub fn dollars(cents: u64) -> String {
    format!("${}.{:02}", cents / 100, cents % 100)
}
EOF
printf '\nFormatting: `format::dollars(1250)` is `$12.50`.\n' >> README.md
printf '# Changes\n\n- Format amounts as dollars.\n- Faster rounding.\n' > CHANGELOG.md
git add -A
git commit -q -m "feat: format amounts as dollars, faster rounding"
