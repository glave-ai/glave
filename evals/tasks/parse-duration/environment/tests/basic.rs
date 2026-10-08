use duration::{parse_duration, ParseError};

#[test]
fn hours_and_minutes() {
    assert_eq!(parse_duration("1h30m"), Ok(5400));
}

#[test]
fn empty_is_an_error() {
    assert_eq!(parse_duration(""), Err(ParseError::Empty));
}
