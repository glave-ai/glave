use squares::SQUARES;

#[test]
fn every_entry_is_a_square() {
    for (n, square) in SQUARES.iter().enumerate() {
        assert_eq!(*square as usize, n * n, "SQUARES[{n}]");
    }
}
