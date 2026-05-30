/// Apply a percentage discount to a price given in cents.
pub fn final_price(cents: u64, discount_percent: u64) -> u64 {
    // BUG: `discount_percent` is not capped at 100, so a larger value makes the
    // discount exceed the price and the subtraction underflows (panics in debug).
    cents - cents * discount_percent / 100
}
