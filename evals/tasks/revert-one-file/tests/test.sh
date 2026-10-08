#!/usr/bin/env bash
# A third commit restores src/lib.rs exactly and keeps the rest of the second.
set -euo pipefail
fail() { echo "$1"; exit 1; }
[ "$(git rev-list --count HEAD)" = 3 ] || fail "expected exactly one new commit"
[ "$(git log -1 --format=%s)" = "fix: restore rounding" ] || fail "wrong message: $(git log -1 --format=%s)"
git diff --quiet HEAD~2 HEAD -- src/lib.rs || fail "src/lib.rs is not as it was before the last commit"
git diff --quiet HEAD~1 HEAD -- README.md CHANGELOG.md src/format.rs || fail "the fix changed other files"
[ -f src/format.rs ] || fail "src/format.rs is gone"
git diff --quiet HEAD || fail "changes are left uncommitted"
