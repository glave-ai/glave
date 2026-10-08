//! Money helpers. Amounts are in cents.

/// `cents` rounded to the nearest whole dollar, halves rounding up: 250 becomes 300.
pub fn round_to_dollars(cents: u64) -> u64 {
    (cents + 50) / 100 * 100
}
