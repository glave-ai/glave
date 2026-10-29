//! What the tally screen shows.

/// How many of `total` are left after `done`, or `None` when more are done than there are.
#[must_use]
pub fn left(done: u64, total: u64) -> Option<u64> {
    total.checked_sub(done)
}
