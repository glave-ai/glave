use temps::{to_celsius, to_fahrenheit};

#[test]
fn water_freezes_at_32_fahrenheit() {
    assert_eq!(to_fahrenheit(0.0), 32.0);
}

#[test]
fn water_boils_at_212_fahrenheit() {
    assert_eq!(to_fahrenheit(100.0), 212.0);
}

#[test]
fn and_back_again() {
    assert_eq!(to_celsius(212.0), 100.0);
}
