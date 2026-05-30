use task::final_price;

#[test]
fn test_no_discount() {
    // PASS_TO_PASS
    assert_eq!(final_price(1000, 0), 1000);
}

#[test]
fn test_half_off() {
    // PASS_TO_PASS
    assert_eq!(final_price(1000, 50), 500);
}

#[test]
fn test_discount_capped_at_100() {
    // FAIL_TO_PASS — a discount over 100% must clamp to free, not underflow.
    assert_eq!(final_price(1000, 150), 0);
}
