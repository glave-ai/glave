//! The settings ledger's record of a person's accent (`specs/model.md`, "The look").
//! `appearance.set`: the accent they chose; the latest replaces every one before it, and with
//! none glave keeps its plain look.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Body, Payload, parse};
use crate::error::InvalidValue;

pub(crate) const SET: &str = "appearance.set";

/// A colour in sRGB, written `#rrggbb`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(try_from = "String", into = "String")]
pub struct Color([u8; 3]);

impl Color {
    /// The colour with these red, green and blue channels.
    pub const fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self([red, green, blue])
    }

    /// The colour `text` writes as `#rrggbb`, in either case.
    pub fn new(text: &str) -> Result<Self, InvalidValue> {
        let invalid = || InvalidValue::new("color", format!("{text:?} is not #rrggbb"));
        let digits = text.strip_prefix('#').ok_or_else(invalid)?;
        if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid());
        }
        let number = u32::from_str_radix(digits, 16).map_err(|_| invalid())?;
        let [_, red, green, blue] = number.to_be_bytes();
        Ok(Self::rgb(red, green, blue))
    }

    /// Its red, green and blue channels.
    pub const fn channels(self) -> [u8; 3] {
        self.0
    }
}

impl TryFrom<String> for Color {
    type Error = InvalidValue;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::new(&text)
    }
}

impl From<Color> for String {
    fn from(color: Color) -> Self {
        color.to_string()
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [red, green, blue] = self.0;
        write!(f, "#{red:02x}{green:02x}{blue:02x}")
    }
}

/// The accent a person chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Accent {
    /// glave's plain look.
    Plain,
    /// A colour, tuned so that text stays easy to read where `adjust` says, else drawn exactly.
    Color { color: Color, adjust: bool },
}

/// The accent a person chose, as they set it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppearanceSet {
    pub accent: Accent,
}

impl Payload for AppearanceSet {
    const TYPE: &'static str = SET;
    const VERSION: u32 = 1;
}

pub(crate) fn decode(
    kind: &str,
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<Body>> {
    (kind == SET && version == 1).then(|| parse(data).map(Body::AppearanceSet))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_color_is_written_as_six_lowercase_digits_and_read_in_either_case() {
        let color = Color::new("#1558D6").unwrap();
        assert_eq!(color.channels(), [0x15, 0x58, 0xd6]);
        assert_eq!(color.to_string(), "#1558d6");
        for wrong in ["1558d6", "#1558d", "#1558d6f", "#15x8d6", "", "#+558d6"] {
            assert!(Color::new(wrong).is_err(), "{wrong}");
        }
    }
}
