use crate::order::{calc_total, Item, Order};

/// A cart that becomes an order at checkout.
#[derive(Debug, Default)]
pub struct Cart {
    order: Order,
}

impl Cart {
    pub fn add(&mut self, name: &str, cents: u64, quantity: u64) {
        self.order.items.push(Item { name: name.to_owned(), cents, quantity });
    }

    /// What the cart costs so far, from `calc_total`.
    pub fn subtotal(&self) -> u64 {
        calc_total(&self.order.items)
    }

    pub fn checkout(self) -> Order {
        self.order
    }
}
