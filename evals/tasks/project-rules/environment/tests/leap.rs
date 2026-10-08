use calendar::is_leap_year;

#[test]
fn leap_years() {
    assert!(is_leap_year(2024));
    assert!(is_leap_year(2000));
    assert!(!is_leap_year(1900));
    assert!(!is_leap_year(2023));
}
