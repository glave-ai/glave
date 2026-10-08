//! Parsing written durations such as `1h30m`. See SPEC.md.

/// Why a duration could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    Empty,
    Invalid,
    Overflow,
}

/// The number of seconds `text` describes, following SPEC.md.
pub fn parse_duration(text: &str) -> Result<u64, ParseError> {
    let _ = text;
    Err(ParseError::Invalid)
}
