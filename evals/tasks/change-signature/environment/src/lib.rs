//! A tiny shop: orders, carts, discounts and invoices. Prices are in cents.

mod cart;
mod discount;
mod invoice;
mod order;

pub use cart::Cart;
pub use discount::discounted_total;
pub use invoice::invoice_line;
pub use order::{calc_total, Item, Order};
