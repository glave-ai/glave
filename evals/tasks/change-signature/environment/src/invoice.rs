use crate::order::{calc_total, Order};

/// One line for an invoice, such as `3 items, $12.50`.
pub fn invoice_line(order: &Order) -> String {
    let count: u64 = order.items.iter().map(|item| item.quantity).sum();
    let cents = calc_total(&order.items);
    format!("{count} items, ${}.{:02}", cents / 100, cents % 100)
}
