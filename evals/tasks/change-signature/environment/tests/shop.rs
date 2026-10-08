use shop::{calc_total, discounted_total, invoice_line, Cart, Item, Order};

fn order() -> Order {
    Order {
        items: vec![
            Item { name: "tea".into(), cents: 450, quantity: 2 },
            Item { name: "cake".into(), cents: 350, quantity: 1 },
        ],
    }
}

#[test]
fn calc_total_adds_price_times_quantity() {
    assert_eq!(calc_total(&order().items), 1250);
}

#[test]
fn an_empty_order_costs_nothing() {
    assert_eq!(calc_total(&[]), 0);
}

#[test]
fn discounts_round_down() {
    assert_eq!(discounted_total(&order(), 10), 1125);
}

#[test]
fn invoices_show_the_count_and_dollars() {
    assert_eq!(invoice_line(&order()), "3 items, $12.50");
}

#[test]
fn carts_add_up() {
    let mut cart = Cart::default();
    cart.add("tea", 450, 2);
    assert_eq!(cart.subtotal(), 900);
}
