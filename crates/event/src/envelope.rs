//! The envelope: the eight members every event shares, and the stored form of an event.

use std::fmt;
use std::num::{NonZeroU32, NonZeroU64};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::error::{DecodeError, EncodeError, InvalidValue};
use crate::id::{EventId, ThreadId};

/// The envelope version this crate reads and writes.
pub(crate) const ENVELOPE_VERSION: u32 = 1;

/// An event's position in its thread, starting at 1 and assigned gap-free by the thread's
/// single writer. This is the order of events; time never is.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(try_from = "u64", into = "u64")]
pub struct Seq(NonZeroU64);

impl Seq {
    pub const FIRST: Self = Self(NonZeroU64::MIN);

    pub fn new(n: u64) -> Result<Self, InvalidValue> {
        NonZeroU64::new(n)
            .map(Self)
            .ok_or_else(|| InvalidValue::new("seq", "must be 1 or more"))
    }

    pub fn get(self) -> u64 {
        self.0.get()
    }

    /// The next position, or `None` if the thread is full (2^64 - 1 events).
    pub fn next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

impl TryFrom<u64> for Seq {
    type Error = InvalidValue;

    fn try_from(n: u64) -> Result<Self, Self::Error> {
        Self::new(n)
    }
}

impl From<Seq> for u64 {
    fn from(seq: Seq) -> Self {
        seq.get()
    }
}

/// The version of one payload type, starting at 1.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(try_from = "u32", into = "u32")]
pub struct PayloadVersion(NonZeroU32);

impl PayloadVersion {
    pub fn new(n: u32) -> Result<Self, InvalidValue> {
        NonZeroU32::new(n)
            .map(Self)
            .ok_or_else(|| InvalidValue::new("payload version", "must be 1 or more"))
    }

    pub fn get(self) -> u32 {
        self.0.get()
    }
}

impl TryFrom<u32> for PayloadVersion {
    type Error = InvalidValue;

    fn try_from(n: u32) -> Result<Self, Self::Error> {
        Self::new(n)
    }
}

impl From<PayloadVersion> for u32 {
    fn from(version: PayloadVersion) -> Self {
        version.get()
    }
}

/// What happened: lowercase dotted segments such as `tool.call.requested`, at most 64 bytes.
/// Each segment is lowercase ASCII letters, digits and underscores; the name starts with a
/// letter.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(try_from = "String", into = "String")]
pub struct TypeName(String);

impl TypeName {
    const MAX_LEN: usize = 64;

    pub fn new(name: impl Into<String>) -> Result<Self, InvalidValue> {
        let name = name.into();
        let starts_with_letter = name.bytes().next().is_some_and(|b| b.is_ascii_lowercase());
        let segments_ok = name.split('.').all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        });
        if name.len() > Self::MAX_LEN || !starts_with_letter || !segments_ok {
            return Err(InvalidValue::new(
                "event type",
                format!(
                    "{name:?} must be lowercase dotted segments of at most {} bytes",
                    Self::MAX_LEN
                ),
            ));
        }
        Ok(Self(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for TypeName {
    type Error = InvalidValue;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        Self::new(name)
    }
}

impl From<TypeName> for String {
    fn from(name: TypeName) -> Self {
        name.0
    }
}

impl fmt::Display for TypeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// When an event was recorded, in UTC at millisecond precision. For people only: events are
/// ordered by [`Seq`]. Always written with three fractional digits, such as
/// `2026-10-07T21:14:01.250Z`, so every instant has exactly one spelling.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(try_from = "String", into = "String")]
pub struct At(Timestamp);

impl At {
    pub fn timestamp(self) -> Timestamp {
        self.0
    }
}

impl TryFrom<String> for At {
    type Error = InvalidValue;

    /// Accepts any RFC 3339 timestamp and truncates it to whole milliseconds.
    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse::<Timestamp>()
            .map(Self::from)
            .map_err(|e| InvalidValue::new("timestamp", format!("{text:?}: {e}")))
    }
}

impl From<At> for String {
    fn from(at: At) -> Self {
        at.to_string()
    }
}

impl fmt::Display for At {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.strftime("%Y-%m-%dT%H:%M:%S%.3fZ"))
    }
}

impl From<Timestamp> for At {
    /// Truncates to whole milliseconds.
    fn from(ts: Timestamp) -> Self {
        Self(Timestamp::from_millisecond(ts.as_millisecond()).unwrap_or(ts))
    }
}

impl From<At> for Timestamp {
    fn from(at: At) -> Self {
        at.0
    }
}

/// The envelope members that do not come from the payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Header {
    pub id: EventId,
    pub thread: ThreadId,
    pub seq: Seq,
    pub at: At,
}

/// An event exactly as it is stored and sent. `data` holds the payload's bytes as written;
/// [`crate::decode`] interprets them.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RawEvent {
    pub glave: u32,
    pub id: EventId,
    pub thread: ThreadId,
    pub seq: Seq,
    pub at: At,
    #[serde(rename = "type")]
    pub kind: TypeName,
    pub v: PayloadVersion,
    /// The payload of `type` at version `v` (`specs/events.md`).
    #[schemars(with = "serde_json::Map<String, serde_json::Value>")]
    pub data: Box<RawValue>,
}

impl RawEvent {
    /// Parses one stored event, such as a line of a JSON Lines export.
    pub fn from_json(text: &str) -> Result<Self, DecodeError> {
        serde_json::from_str(text).map_err(|source| DecodeError::Envelope { source })
    }

    /// Writes the event as one line of JSON, members in specification order.
    pub fn to_json(&self) -> Result<String, EncodeError> {
        serde_json::to_string(self).map_err(|source| EncodeError::Serialize {
            kind: "envelope",
            source,
        })
    }

    pub fn header(&self) -> Header {
        Header {
            id: self.id,
            thread: self.thread,
            seq: self.seq,
            at: self.at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seq_starts_at_one_and_counts_up() {
        assert!(Seq::new(0).is_err());
        assert_eq!(Seq::FIRST.get(), 1);
        assert_eq!(Seq::FIRST.next().map(Seq::get), Some(2));
        assert_eq!(Seq::new(u64::MAX).unwrap().next(), None);
        assert!(serde_json::from_str::<Seq>("0").is_err());
    }

    #[test]
    fn type_names_are_lowercase_dotted() {
        for good in [
            "thread.started",
            "tool.call.requested",
            "model.responded",
            "a",
            "x1.y_2",
        ] {
            assert!(TypeName::new(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            "Thread.started",
            "tool..call",
            ".tool",
            "tool.",
            "1tool",
            "tool call",
            "tool-call",
        ] {
            assert!(TypeName::new(bad).is_err(), "{bad}");
        }
        assert!(TypeName::new("a".repeat(65)).is_err());
    }

    #[test]
    fn at_truncates_to_milliseconds() {
        let precise: Timestamp = "2026-10-07T21:14:03.512987Z".parse().unwrap();
        let at = At::from(precise);
        assert_eq!(at.to_string(), "2026-10-07T21:14:03.512Z");
        assert_eq!(
            serde_json::to_string(&at).unwrap(),
            "\"2026-10-07T21:14:03.512Z\""
        );
    }

    #[test]
    fn at_always_writes_three_fractional_digits() {
        for (input, written) in [
            ("2026-10-07T21:14:01.25Z", "2026-10-07T21:14:01.250Z"),
            ("2026-10-07T21:14:01Z", "2026-10-07T21:14:01.000Z"),
            ("2026-10-07T23:14:01.5+02:00", "2026-10-07T21:14:01.500Z"),
        ] {
            let at: At = serde_json::from_str(&format!("\"{input}\"")).unwrap();
            assert_eq!(at.to_string(), written, "{input}");
        }
        assert!(serde_json::from_str::<At>("\"yesterday\"").is_err());
    }
}
