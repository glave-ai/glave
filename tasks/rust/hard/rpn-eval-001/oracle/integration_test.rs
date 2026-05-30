use task::eval_rpn;

#[test]
fn test_addition() {
    // PASS_TO_PASS — commutative, so operand order doesn't matter.
    assert_eq!(eval_rpn(&["2", "3", "+"]), 5);
}

#[test]
fn test_multiplication() {
    // PASS_TO_PASS — commutative.
    assert_eq!(eval_rpn(&["4", "5", "*"]), 20);
}

#[test]
fn test_subtraction_order() {
    // FAIL_TO_PASS — "10 3 -" must be 10 - 3 = 7, not 3 - 10.
    assert_eq!(eval_rpn(&["10", "3", "-"]), 7);
}

#[test]
fn test_division_order() {
    // FAIL_TO_PASS — "20 4 /" must be 20 / 4 = 5, not 4 / 20.
    assert_eq!(eval_rpn(&["20", "4", "/"]), 5);
}
