#!/usr/bin/env bash
# The function is right, and it follows every rule in CONTRIBUTING.md.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
fail() { echo "$1"; exit 1; }
grep -q 'pub fn days_in_month' src/lib.rs || fail "no pub fn days_in_month in src/lib.rs"
grep -B1 'pub fn days_in_month' src/lib.rs | head -1 | grep -q '^ *///' \
    || fail "days_in_month has no /// doc comment directly above it"
! grep -rnE 'unwrap\(|expect\(|panic!' src || fail "src/ uses unwrap, expect or panic!"
! grep -rnE '#\[(cfg\(test\)|test)\]' src || fail "src/ has tests in it"
grep -lq days_in_month tests/*.rs || fail "no test in tests/ uses days_in_month"
cp "$here/months.rs" tests/hidden_months.rs
cargo test --quiet
