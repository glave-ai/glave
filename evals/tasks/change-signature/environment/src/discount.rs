use crate::order::{calc_total, Order};

/// The order's total after taking `percent` off, rounded down to the cent.
pub fn discounted_total(order: &Order, percent: u64) -> u64 {
    let total = calc_total(&order.items);
    total - total * percent.min(100) / 100
}
