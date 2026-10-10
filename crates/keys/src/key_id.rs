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

/// Names one data key: the space it belongs to, its month, and which of that month's keys
/// it is (a month gets a new key after its key is destroyed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyId {
    pub space: SpaceId,
    pub month: Month,
    pub n: u32,
}

/// The prefix that names the format: XChaCha20-Poly1305 under per-month data keys. A new
/// algorithm gets a new prefix, never a migration.
const VERSION: &str = "v1";

impl fmt::Display for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{VERSION}/{}/{}/{}", self.space, self.month, self.n)
    }
}

impl FromStr for KeyId {
    type Err = KeyError;

    fn from_str(text: &str) -> Result<Self, KeyError> {
        let invalid = || KeyError::InvalidKeyId(text.to_owned());
        let mut parts = text.split('/');
        let (Some(VERSION), Some(space), Some(month), Some(n), None) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) else {
            return Err(invalid());
        };
        let space = Uuid::parse_str(space).map_err(|_| invalid())?;
        let n: u32 = n.parse().map_err(|_| invalid())?;
        if n == 0 {
            return Err(invalid());
        }
        Ok(Self {
            space: SpaceId::from_uuid(space),
            month: month.parse().map_err(|_| invalid())?,
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
            month: "2026-10".parse().unwrap(),
            n: 2,
        };
        let text = id.to_string();
        assert_eq!(text, "v1/00000000-0000-0000-0000-000000000005/2026-10/2");
        assert_eq!(text.parse::<KeyId>().unwrap(), id);
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
