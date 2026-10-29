#!/usr/bin/env bash
set -euo pipefail
app=packages/app
cat >> "$app/src/lib.rs" <<'RS'

/// The share `part` is of `whole`, as a whole percent rounded down, or `None` when `whole` is 0
/// or `part` is more than `whole`.
#[must_use]
pub fn percent(part: u64, whole: u64) -> Option<u8> {
    if whole == 0 || part > whole {
        return None;
    }
    let share = u128::from(part) * 100 / u128::from(whole);
    u8::try_from(share).ok()
}
RS
cat > "$app/tests/percent.rs" <<'RS'
use app::percent;

#[test]
fn a_third_rounds_down() {
    assert_eq!(percent(1, 3), Some(33));
    assert_eq!(percent(1, 0), None);
}
RS
awk '{ print } /^## Unreleased$/ { print ""; print "- Added `percent`." }' "$app/CHANGELOG.md" \
    > "$app/CHANGELOG.new"
mv -f "$app/CHANGELOG.new" "$app/CHANGELOG.md"
