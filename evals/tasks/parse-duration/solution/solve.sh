#!/usr/bin/env bash
set -euo pipefail
cat > src/lib.rs <<'EOF'
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
    let text = text.trim_matches(' ');
    if text.is_empty() {
        return Err(ParseError::Empty);
    }
    let mut chars = text.chars().peekable();
    let (mut total, mut last) = (0u64, 0u8);
    while chars.peek().is_some() {
        while chars.peek() == Some(&' ') {
            chars.next();
        }
        let mut digits = String::new();
        while let Some(c) = chars.peek().copied().filter(char::is_ascii_digit) {
            digits.push(c);
            chars.next();
        }
        if digits.is_empty() {
            return Err(ParseError::Invalid);
        }
        let (rank, unit) = match chars.next() {
            Some('d') => (1, 86_400),
            Some('h') => (2, 3_600),
            Some('m') => (3, 60),
            Some('s') => (4, 1),
            _ => return Err(ParseError::Invalid),
        };
        if rank <= last {
            return Err(ParseError::Invalid);
        }
        last = rank;
        let number: u64 = digits.parse().map_err(|_| ParseError::Overflow)?;
        let seconds = number.checked_mul(unit).ok_or(ParseError::Overflow)?;
        total = total.checked_add(seconds).ok_or(ParseError::Overflow)?;
    }
    Ok(total)
}
EOF
