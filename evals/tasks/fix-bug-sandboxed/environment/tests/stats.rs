use stats::{mean, median};

#[test]
fn mean_of_values() {
    assert_eq!(mean(&[1.0, 2.0, 3.0, 4.0]), Some(2.5));
}

#[test]
fn median_of_an_odd_count() {
    assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
}

#[test]
fn median_of_an_even_count() {
    assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), Some(2.5));
}

#[test]
fn empty_input() {
    assert_eq!(mean(&[]), None);
    assert_eq!(median(&[]), None);
}
