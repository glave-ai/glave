use shop::{order_total, Item, Order};

#[test]
fn order_total_takes_an_order() {
    let order = Order { items: vec![Item { name: "pen".into(), cents: 120, quantity: 3 }] };
    assert_eq!(order_total(&order), 360);
    assert_eq!(order_total(&Order::default()), 0);
}
