//! Naming data keys: `v1/<space>/<YYYY-MM>/<n>`, the `key_id` stored beside sealed content.

use std::fmt;
use std::str::FromStr;

use glave_ports::SpaceId;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::KeyError;

/// A calendar month in UTC: the period one data key covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Month {
    year: i16,
    month: i8,
}

impl Month {
    /// The month `at` falls in, in UTC.
    pub fn of(at: Timestamp) -> Self {
        let date = at.to_zoned(TimeZone::UTC).date();
        Self {
            year: date.year(),
            month: date.month(),
        }
    }
}

impl fmt::Display for Month {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}", self.year, self.month)
    }
}

impl FromStr for Month {
    type Err = KeyError;

    fn from_str(text: &str) -> Result<Self, KeyError> {
        let invalid = || KeyError::InvalidKeyId(format!("month {text:?}"));
        let (year, month) = text.split_once('-').ok_or_else(invalid)?;
        if year.len() != 4 || month.len() != 2 {
            return Err(invalid());
        }
        let year: i16 = year.parse().map_err(|_| invalid())?;
        let month: i8 = month.parse().map_err(|_| invalid())?;
        if !(1..=12).contains(&month) || year < 0 {
            return Err(invalid());
        }
        Ok(Self { year, month })
    }
}

impl TryFrom<String> for Month {
    type Error = KeyError;

    fn try_from(text: String) -> Result<Self, KeyError> {
        text.parse()
    }
}

impl From<Month> for String {
    fn from(month: Month) -> Self {
        month.to_string()
    }
}

/// What one data key seals: a month of the space's content, or one item with a key of its own
/// (`specs/memory.md`: each memory, so forgetting it erases every version at once; and each
/// model account a person connects, so removing it erases its key).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Period {
    Month(Month),
    Memory(Uuid),
    Provider(Uuid),
}

impl fmt::Display for Period {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Month(month) => month.fmt(f),
            Self::Memory(memory) => write!(f, "memory/{memory}"),
            Self::Provider(provider) => write!(f, "provider/{provider}"),
        }
    }
}

/// Names one data key: the space it belongs to, what it seals, and which of that period's keys
/// it is (a month gets a new key after its key is destroyed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyId {
    pub space: SpaceId,
    pub period: Period,
    pub n: u32,
}

/// The prefix that names the format: XChaCha20-Poly1305 under per-month data keys. A new
/// algorithm gets a new prefix, never a migration.
const VERSION: &str = "v1";

impl fmt::Display for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{VERSION}/{}/{}/{}", self.space, self.period, self.n)
    }
}

impl FromStr for KeyId {
    type Err = KeyError;

    /// `v1/<space>/<YYYY-MM>/<n>`, `v1/<space>/memory/<memory>/<n>` or
    /// `v1/<space>/provider/<provider>/<n>`.
    fn from_str(text: &str) -> Result<Self, KeyError> {
        let invalid = || KeyError::InvalidKeyId(text.to_owned());
        let parts: Vec<&str> = text.split('/').collect();
        let (space, period, n) = match parts.as_slice() {
            [VERSION, space, month, n] => (*space, Period::Month(month.parse()?), *n),
            [VERSION, space, "memory", memory, n] => (
                *space,
                Period::Memory(Uuid::parse_str(memory).map_err(|_| invalid())?),
                *n,
            ),
            [VERSION, space, "provider", provider, n] => (
                *space,
                Period::Provider(Uuid::parse_str(provider).map_err(|_| invalid())?),
                *n,
            ),
            _ => return Err(invalid()),
        };
        let space = Uuid::parse_str(space).map_err(|_| invalid())?;
        let n: u32 = n.parse().map_err(|_| invalid())?;
        if n == 0 {
            return Err(invalid());
        }
        Ok(Self {
            space: SpaceId::from_uuid(space),
            period,
            n,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_id_round_trips_through_its_text() {
        let id = KeyId {
            space: SpaceId::from_uuid(Uuid::from_u128(5)),
            period: Period::Month("2026-10".parse().unwrap()),
            n: 2,
        };
        let text = id.to_string();
        assert_eq!(text, "v1/00000000-0000-0000-0000-000000000005/2026-10/2");
        assert_eq!(text.parse::<KeyId>().unwrap(), id);
        let memory = KeyId {
            period: Period::Memory(Uuid::from_u128(9)),
            n: 1,
            ..id
        };
        let text = memory.to_string();
        assert_eq!(
            text,
            "v1/00000000-0000-0000-0000-000000000005/memory/00000000-0000-0000-0000-000000000009/1"
        );
        assert_eq!(text.parse::<KeyId>().unwrap(), memory);
        let provider = KeyId {
            period: Period::Provider(Uuid::from_u128(9)),
            ..memory
        };
        let text = provider.to_string();
        assert!(text.ends_with("/provider/00000000-0000-0000-0000-000000000009/1"));
        assert_eq!(text.parse::<KeyId>().unwrap(), provider);
    }

    #[test]
    fn malformed_key_ids_are_refused() {
        for text in [
            "",
            "v2/00000000-0000-0000-0000-000000000005/2026-10/1",
            "v1/not-a-uuid/2026-10/1",
            "v1/00000000-0000-0000-0000-000000000005/2026-13/1",
            "v1/00000000-0000-0000-0000-000000000005/2026-10/0",
            "v1/00000000-0000-0000-0000-000000000005/2026-10/1/extra",
            "v1/00000000-0000-0000-0000-000000000005/memory/nope/1",
            "v1/00000000-0000-0000-0000-000000000005/notes/00000000-0000-0000-0000-000000000009/1",
        ] {
            assert!(text.parse::<KeyId>().is_err(), "{text}");
        }
    }

    #[test]
    fn the_month_is_taken_in_utc() {
        let at: Timestamp = "2026-10-31T23:30:00-02:00".parse().unwrap();
        assert_eq!(Month::of(at).to_string(), "2026-11");
    }
}
