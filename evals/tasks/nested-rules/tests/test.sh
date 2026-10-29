#!/usr/bin/env bash
# The function is right, and it follows every rule in packages/app/AGENTS.md.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
fail() { echo "$1"; exit 1; }
app=packages/app
grep -q 'pub fn percent' "$app/src/lib.rs" || fail "no pub fn percent in $app/src/lib.rs"
above="$(grep -B4 'pub fn percent' "$app/src/lib.rs")"
grep -q '#\[must_use\]' <<<"$above" || fail "percent is not #[must_use]"
grep -q '^ *///' <<<"$above" || fail "percent has no /// doc comment"
! grep -rnE 'unwrap\(|expect\(|panic!' "$app/src" || fail "$app/src uses unwrap, expect or panic!"
! grep -rnE '#\[(cfg\(test\)|test)\]' "$app/src" || fail "$app/src has tests in it"
[ -f "$app/tests/percent.rs" ] || fail "no $app/tests/percent.rs"
grep -q percent "$app/tests/percent.rs" || fail "$app/tests/percent.rs does not test percent"
sed -n '/^## Unreleased/,/^## 0.1.0/p' "$app/CHANGELOG.md" | grep -q 'Added `percent`' \
    || fail "CHANGELOG.md has no line for percent under ## Unreleased"
cp "$here/percent.rs" "$app/tests/hidden_percent.rs"
cargo test --quiet -p app
