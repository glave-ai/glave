//! Identifiers. Events and threads use UUID v7; tool calls use a short validated string.

use std::fmt;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::{Builder, Uuid};

use crate::error::InvalidValue;

/// Builds a UUID v7 from a time and caller-supplied randomness, so ids are deterministic in
/// tests and the clock and random source stay injectable.
fn build_v7(what: &'static str, at: Timestamp, random: [u8; 10]) -> Result<Uuid, InvalidValue> {
    let millis = u64::try_from(at.as_millisecond())
        .map_err(|_| InvalidValue::new(what, format!("{at} is before 1970")))?;
    Ok(Builder::from_unix_timestamp_millis(millis, &random).into_uuid())
}

fn require_v7(what: &'static str, uuid: Uuid) -> Result<Uuid, InvalidValue> {
    match uuid.get_version_num() {
        7 => Ok(uuid),
        other => Err(InvalidValue::new(
            what,
            format!("{uuid} is a version {other} UUID; version 7 is required"),
        )),
    }
}

/// Identifies one event, unique everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "Uuid", into = "Uuid")]
pub struct EventId(Uuid);

impl EventId {
    pub fn new(at: Timestamp, random: [u8; 10]) -> Result<Self, InvalidValue> {
        build_v7("event id", at, random).map(Self)
    }

    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl TryFrom<Uuid> for EventId {
    type Error = InvalidValue;

    fn try_from(uuid: Uuid) -> Result<Self, Self::Error> {
        require_v7("event id", uuid).map(Self)
    }
}

impl From<EventId> for Uuid {
    fn from(id: EventId) -> Self {
        id.0
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Identifies one thread: a conversation or task with its own ordered log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "Uuid", into = "Uuid")]
pub struct ThreadId(Uuid);

impl ThreadId {
    pub fn new(at: Timestamp, random: [u8; 10]) -> Result<Self, InvalidValue> {
        build_v7("thread id", at, random).map(Self)
    }

    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl TryFrom<Uuid> for ThreadId {
    type Error = InvalidValue;

    fn try_from(uuid: Uuid) -> Result<Self, Self::Error> {
        require_v7("thread id", uuid).map(Self)
    }
}

impl From<ThreadId> for Uuid {
    fn from(id: ThreadId) -> Self {
        id.0
    }
}

impl fmt::Display for ThreadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Links a tool call to its result: 1 to 128 printable ASCII characters.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CallId(String);

impl CallId {
    const MAX_LEN: usize = 128;

    pub fn new(id: impl Into<String>) -> Result<Self, InvalidValue> {
        let id = id.into();
        if id.is_empty() || id.len() > Self::MAX_LEN {
            return Err(InvalidValue::new(
                "call id",
                format!(
                    "must be 1 to {} characters, got {}",
                    Self::MAX_LEN,
                    id.len()
                ),
            ));
        }
        if !id.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(InvalidValue::new(
                "call id",
                "must be printable ASCII without spaces",
            ));
        }
        Ok(Self(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CallId {
    type Error = InvalidValue;

    fn try_from(id: String) -> Result<Self, Self::Error> {
        Self::new(id)
    }
}

impl From<CallId> for String {
    fn from(id: CallId) -> Self {
        id.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    #[test]
    fn ids_are_version_7_and_deterministic() {
        let a = EventId::new(at("2026-10-07T21:14:03.512Z"), [7; 10]).unwrap();
        let b = EventId::new(at("2026-10-07T21:14:03.512Z"), [7; 10]).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.as_uuid().get_version_num(), 7);
    }

    #[test]
    fn ids_sort_by_time() {
        let earlier = ThreadId::new(at("2026-10-07T21:14:03.512Z"), [9; 10]).unwrap();
        let later = ThreadId::new(at("2026-10-07T21:14:03.513Z"), [0; 10]).unwrap();
        assert!(earlier < later);
    }

    #[test]
    fn rejects_other_uuid_versions() {
        let v4: Uuid = "5f0c8a4e-2b1d-4c3a-9e7f-1a2b3c4d5e6f".parse().unwrap();
        assert!(EventId::try_from(v4).is_err());
        assert!(
            serde_json::from_str::<ThreadId>("\"5f0c8a4e-2b1d-4c3a-9e7f-1a2b3c4d5e6f\"").is_err()
        );
    }

    #[test]
    fn rejects_times_before_1970() {
        assert!(EventId::new(at("1969-12-31T23:59:59Z"), [0; 10]).is_err());
    }

    #[test]
    fn call_ids_are_validated() {
        assert!(CallId::new("toolu_01A").is_ok());
        assert!(CallId::new("").is_err());
        assert!(CallId::new("has space").is_err());
        assert!(CallId::new("x".repeat(129)).is_err());
    }
}
