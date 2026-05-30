/// Apply a percentage discount to a price given in cents.
pub fn final_price(cents: u64, discount_percent: u64) -> u64 {
    let pct = discount_percent.min(100);
    cents - cents * pct / 100
}
