use calendar::days_in_month;

#[test]
fn long_and_short_months() {
    for month in [1, 3, 5, 7, 8, 10, 12] {
        assert_eq!(days_in_month(2023, month), Some(31), "month {month}");
    }
    for month in [4, 6, 9, 11] {
        assert_eq!(days_in_month(2023, month), Some(30), "month {month}");
    }
}

#[test]
fn february() {
    assert_eq!(days_in_month(2024, 2), Some(29));
    assert_eq!(days_in_month(2000, 2), Some(29));
    assert_eq!(days_in_month(1900, 2), Some(28));
    assert_eq!(days_in_month(2023, 2), Some(28));
}

#[test]
fn months_outside_the_year() {
    assert_eq!(days_in_month(2023, 0), None);
    assert_eq!(days_in_month(2023, 13), None);
}
