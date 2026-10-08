/// One line of an order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub name: String,
    pub cents: u64,
    pub quantity: u64,
}

/// An order: the items someone is buying.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Order {
    pub items: Vec<Item>,
}

/// The total of `items` in cents: each item's price times its quantity.
pub fn calc_total(items: &[Item]) -> u64 {
    items.iter().map(|item| item.cents * item.quantity).sum()
}
