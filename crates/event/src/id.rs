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

/// A UUID v7 id type, `$what` in what its errors say: made from a time and caller-supplied
/// randomness, read only from a version 7 UUID, and shown as the UUID.
macro_rules! v7_id {
    ($(#[$doc:meta])* $name:ident, $what:literal) => {
        $(#[$doc])*
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
        #[serde(try_from = "Uuid", into = "Uuid")]
        pub struct $name(Uuid);

        impl $name {
            pub fn new(at: Timestamp, random: [u8; 10]) -> Result<Self, InvalidValue> {
                build_v7($what, at, random).map(Self)
            }

            pub fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl TryFrom<Uuid> for $name {
            type Error = InvalidValue;

            fn try_from(uuid: Uuid) -> Result<Self, Self::Error> {
                require_v7($what, uuid).map(Self)
            }
        }

        impl From<$name> for Uuid {
            fn from(id: $name) -> Self {
                id.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

v7_id!(
    /// Identifies one event, unique everywhere.
    EventId,
    "event id"
);

v7_id!(
    /// Identifies a principal: a person, an agent profile or a group. People, agents and groups
    /// share one id space, unique everywhere and never tied to one deployment.
    PrincipalId,
    "principal id"
);

/// Identifies one thread: a conversation or task with its own ordered log.
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
#[serde(try_from = "Uuid", into = "Uuid")]
pub struct ThreadId(Uuid);

impl ThreadId {
    pub fn new(at: Timestamp, random: [u8; 10]) -> Result<Self, InvalidValue> {
        build_v7("thread id", at, random).map(Self)
    }

    /// The id at time zero whose random bits are all zero but the last byte, `last`, such as
    /// `00000000-0000-7000-8000-000000000002`: an id no thread can have, since a thread's id
    /// carries the time it started, kept for glave's own streams.
    pub fn reserved(last: u8) -> Self {
        let mut random = [0u8; 10];
        if let Some(end) = random.last_mut() {
            *end = last;
        }
        Self(Builder::from_unix_timestamp_millis(0, &random).into_uuid())
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

v7_id!(
    /// Identifies one schedule: a prompt or goal that runs as a task at set times.
    ScheduleId,
    "schedule id"
);

v7_id!(
    /// Identifies one invite to an organization (`specs/model.md`, "Invites").
    InviteId,
    "invite id"
);

v7_id!(
    /// Identifies one memory: a short statement kept for later threads (`specs/memory.md`).
    MemoryId,
    "memory id"
);

impl std::str::FromStr for MemoryId {
    type Err = InvalidValue;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let uuid = Uuid::parse_str(text.trim())
            .map_err(|e| InvalidValue::new("memory id", format!("{text:?}: {e}")))?;
        Self::try_from(uuid)
    }
}

v7_id!(
    /// Identifies one model account a person connected (`provider.connected`).
    ProviderId,
    "provider id"
);

impl std::str::FromStr for ProviderId {
    type Err = InvalidValue;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let uuid = Uuid::parse_str(text.trim())
            .map_err(|e| InvalidValue::new("provider id", format!("{text:?}: {e}")))?;
        Self::try_from(uuid)
    }
}

v7_id!(
    /// Identifies one sign-in or secret a person keeps, sealed under a key of its own: a git
    /// host (`credential.connected`), a connection (`connection.connected`) or a secret
    /// (`secret.added`).
    CredentialId,
    "credential id"
);

/// Links a tool call to its result: 1 to 128 printable ASCII characters.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, schemars::JsonSchema,
)]
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
