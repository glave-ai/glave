use app::left;

#[test]
fn what_is_left() {
    assert_eq!(left(3, 10), Some(7));
    assert_eq!(left(11, 10), None);
}
