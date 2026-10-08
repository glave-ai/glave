use duration::parse_duration;
use duration::ParseError::{Empty, Invalid, Overflow};

#[test]
fn single_units() {
    assert_eq!(parse_duration("45s"), Ok(45));
    assert_eq!(parse_duration("2h"), Ok(7_200));
    assert_eq!(parse_duration("1d"), Ok(86_400));
    assert_eq!(parse_duration("0s"), Ok(0));
}

#[test]
fn every_unit_in_order() {
    assert_eq!(parse_duration("1d2h3m4s"), Ok(93_784));
    assert_eq!(parse_duration("1d4s"), Ok(86_404));
}

#[test]
fn spaces_between_parts_and_around() {
    assert_eq!(parse_duration("1h 30m"), Ok(5_400));
    assert_eq!(parse_duration("  2m  "), Ok(120));
    assert_eq!(parse_duration("1d   1s"), Ok(86_401));
}

#[test]
fn no_space_inside_a_part() {
    assert_eq!(parse_duration("1 h"), Err(Invalid));
    assert_eq!(parse_duration("1h 3 0m"), Err(Invalid));
}

#[test]
fn leading_zeros() {
    assert_eq!(parse_duration("05m"), Ok(300));
    assert_eq!(parse_duration("000s"), Ok(0));
}

#[test]
fn units_in_order_and_once() {
    assert_eq!(parse_duration("30m1h"), Err(Invalid));
    assert_eq!(parse_duration("1h1h"), Err(Invalid));
    assert_eq!(parse_duration("1s1m"), Err(Invalid));
}

#[test]
fn malformed_parts() {
    for text in ["1H", "1x", "h", "1", "-1s", "+1s", "1.5h", "1h,30m", "1h30"] {
        assert_eq!(parse_duration(text), Err(Invalid), "{text:?}");
    }
}

#[test]
fn blank_text_is_empty() {
    assert_eq!(parse_duration("   "), Err(Empty));
}

#[test]
fn numbers_and_totals_that_overflow() {
    assert_eq!(parse_duration("18446744073709551615s"), Ok(u64::MAX));
    assert_eq!(parse_duration("18446744073709551616s"), Err(Overflow));
    assert_eq!(parse_duration("213503982334602d"), Err(Overflow));
    assert_eq!(parse_duration("213503982334601d25215s"), Ok(u64::MAX));
    assert_eq!(parse_duration("213503982334601d25216s"), Err(Overflow));
}
