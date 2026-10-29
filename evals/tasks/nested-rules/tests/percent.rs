use app::percent;

#[test]
fn shares_round_down() {
    assert_eq!(percent(1, 3), Some(33));
    assert_eq!(percent(2, 3), Some(66));
    assert_eq!(percent(0, 7), Some(0));
    assert_eq!(percent(7, 7), Some(100));
    assert_eq!(percent(u64::MAX, u64::MAX), Some(100));
    assert_eq!(percent(u64::MAX / 2, u64::MAX), Some(49));
}

#[test]
fn no_share_of_nothing_or_more_than_all() {
    assert_eq!(percent(0, 0), None);
    assert_eq!(percent(8, 7), None);
}
