use inventory::Stock;

#[test]
fn adding_twice_adds_up() {
    let mut stock = Stock::new();
    stock.add("pen", 2);
    stock.add("pen", 3);
    assert_eq!(stock.count("pen"), 5);
}

#[test]
fn taking_what_is_in_stock() {
    let mut stock = Stock::new();
    stock.add("pen", 5);
    assert!(stock.take("pen", 2));
    assert_eq!(stock.count("pen"), 3);
}

#[test]
fn taking_more_than_is_in_stock_takes_nothing() {
    let mut stock = Stock::new();
    stock.add("pen", 1);
    assert!(!stock.take("pen", 2));
    assert_eq!(stock.count("pen"), 1);
}

#[test]
fn an_item_never_added_is_not_in_stock() {
    let mut stock = Stock::new();
    assert_eq!(stock.count("ink"), 0);
    assert!(!stock.take("ink", 1));
}
