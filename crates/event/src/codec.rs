//! Encoding payloads into events, and decoding stored events into the latest version of each
//! payload. Events this reader does not understand are kept untouched, never dropped.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::value::RawValue;

use crate::envelope::{ENVELOPE_VERSION, Header, PayloadVersion, RawEvent, TypeName};
use crate::error::{DecodeError, EncodeError};
use crate::payload::{
    ApprovalDecided, ApprovalRequested, GoalEnded, GoalResumed, GoalSet, HookRan, MessageAdded,
    ModelResponded, ScheduleCreated, ScheduleDeleted, TaskEnded, TaskStarted, ThreadArchived,
    ThreadCompacted, ThreadDeleted, ThreadStarted, ThreadUnarchived, ToolCallCompleted,
    ToolCallRequested, TurnEnded, TurnStarted, WorkspaceBound, WorkspaceRemoved, approval, goal,
    hook, message, model, standing, task, thread, tool, turn, workspace,
};

/// A payload type: the data of one event type at its latest version.
pub trait Payload: Serialize + DeserializeOwned {
    /// The event type this payload is recorded under.
    const TYPE: &'static str;
    /// The latest version. Writers always write it; older versions are upgraded on read.
    const VERSION: u32;
}

/// Every (type, version) pair this crate can read, including old versions. Each has a saved
/// example in `fixtures/<type>/v<version>.json`; the fixture tests fail if one is missing.
pub const KNOWN_VERSIONS: &[(&str, u32)] = &[
    (thread::TYPE, 1),
    (thread::TYPE, 2),
    (thread::TYPE, 3),
    (thread::COMPACTED, 1),
    (standing::ARCHIVED, 1),
    (standing::UNARCHIVED, 1),
    (standing::DELETED, 1),
    (turn::STARTED, 1),
    (turn::STARTED, 2),
    (message::TYPE, 1),
    (message::TYPE, 2),
    (tool::REQUESTED, 1),
    (tool::COMPLETED, 1),
    (model::TYPE, 1),
    (model::TYPE, 2),
    (turn::ENDED, 1),
    (approval::REQUESTED, 1),
    (approval::DECIDED, 1),
    (approval::DECIDED, 2),
    (goal::SET, 1),
    (goal::SET, 2),
    (goal::ENDED, 1),
    (goal::ENDED, 2),
    (goal::RESUMED, 1),
    (workspace::BOUND, 1),
    (workspace::REMOVED, 1),
    (hook::TYPE, 1),
    (task::STARTED, 1),
    (task::ENDED, 1),
    (task::CREATED, 1),
    (task::DELETED, 1),
];

/// A decoded event: its header and its payload at the latest version.
#[derive(Debug, Clone)]
pub struct Event {
    pub header: Header,
    pub body: Body,
}

/// The payload of a decoded event.
#[derive(Debug, Clone)]
pub enum Body {
    ThreadStarted(ThreadStarted),
    ThreadCompacted(ThreadCompacted),
    ThreadArchived(ThreadArchived),
    ThreadUnarchived(ThreadUnarchived),
    ThreadDeleted(ThreadDeleted),
    TurnStarted(TurnStarted),
    MessageAdded(MessageAdded),
    ToolCallRequested(ToolCallRequested),
    ToolCallCompleted(ToolCallCompleted),
    ModelResponded(ModelResponded),
    TurnEnded(TurnEnded),
    ApprovalRequested(ApprovalRequested),
    ApprovalDecided(ApprovalDecided),
    GoalSet(GoalSet),
    GoalEnded(GoalEnded),
    GoalResumed(GoalResumed),
    WorkspaceBound(WorkspaceBound),
    WorkspaceRemoved(WorkspaceRemoved),
    HookRan(HookRan),
    TaskStarted(TaskStarted),
    TaskEnded(TaskEnded),
    ScheduleCreated(ScheduleCreated),
    ScheduleDeleted(ScheduleDeleted),
    /// An event this reader does not understand: an unknown type, or a newer version of a
    /// known one. Kept exactly as written.
    Unknown(Unknown),
}

/// An event kept untouched because this reader does not understand it.
#[derive(Debug, Clone)]
pub struct Unknown {
    pub kind: TypeName,
    pub v: PayloadVersion,
    pub data: Box<RawValue>,
}

/// Encodes a payload at its latest version into an event ready to store or send.
pub fn encode<P: Payload>(header: Header, payload: &P) -> Result<RawEvent, EncodeError> {
    let type_info = |source| EncodeError::TypeInfo {
        kind: P::TYPE,
        version: P::VERSION,
        source,
    };
    let kind = TypeName::new(P::TYPE).map_err(type_info)?;
    let v = PayloadVersion::new(P::VERSION).map_err(type_info)?;
    let data =
        serde_json::value::to_raw_value(payload).map_err(|source| EncodeError::Serialize {
            kind: P::TYPE,
            source,
        })?;
    Ok(RawEvent {
        glave: ENVELOPE_VERSION,
        id: header.id,
        thread: header.thread,
        seq: header.seq,
        at: header.at,
        kind,
        v,
        data,
    })
}

/// Decodes a stored event, upgrading an old payload version to the latest. Unknown types and
/// newer versions come back as [`Body::Unknown`] with their bytes untouched.
pub fn decode(raw: RawEvent) -> Result<Event, DecodeError> {
    if raw.glave != ENVELOPE_VERSION {
        return Err(DecodeError::UnsupportedEnvelope {
            id: raw.id,
            found: raw.glave,
            supported: ENVELOPE_VERSION,
        });
    }
    let header = raw.header();
    let version = raw.v.get();
    let data = &raw.data;
    let known = match raw.kind.as_str() {
        thread::TYPE => thread::decode(version, data).map(|r| r.map(Body::ThreadStarted)),
        thread::COMPACTED => {
            thread::decode_compacted(version, data).map(|r| r.map(Body::ThreadCompacted))
        }
        standing::ARCHIVED => {
            standing::decode_archived(version, data).map(|r| r.map(Body::ThreadArchived))
        }
        standing::UNARCHIVED => {
            standing::decode_unarchived(version, data).map(|r| r.map(Body::ThreadUnarchived))
        }
        standing::DELETED => {
            standing::decode_deleted(version, data).map(|r| r.map(Body::ThreadDeleted))
        }
        turn::STARTED => turn::decode_started(version, data).map(|r| r.map(Body::TurnStarted)),
        message::TYPE => message::decode(version, data).map(|r| r.map(Body::MessageAdded)),
        tool::REQUESTED => {
            tool::decode_requested(version, data).map(|r| r.map(Body::ToolCallRequested))
        }
        tool::COMPLETED => {
            tool::decode_completed(version, data).map(|r| r.map(Body::ToolCallCompleted))
        }
        model::TYPE => model::decode(version, data).map(|r| r.map(Body::ModelResponded)),
        turn::ENDED => turn::decode_ended(version, data).map(|r| r.map(Body::TurnEnded)),
        approval::REQUESTED => {
            approval::decode_requested(version, data).map(|r| r.map(Body::ApprovalRequested))
        }
        approval::DECIDED => {
            approval::decode_decided(version, data).map(|r| r.map(Body::ApprovalDecided))
        }
        goal::SET => goal::decode_set(version, data).map(|r| r.map(Body::GoalSet)),
        goal::ENDED => goal::decode_ended(version, data).map(|r| r.map(Body::GoalEnded)),
        goal::RESUMED => goal::decode_resumed(version, data).map(|r| r.map(Body::GoalResumed)),
        workspace::BOUND => workspace::decode(version, data).map(|r| r.map(Body::WorkspaceBound)),
        workspace::REMOVED => {
            workspace::decode_removed(version, data).map(|r| r.map(Body::WorkspaceRemoved))
        }
        hook::TYPE => hook::decode(version, data).map(|r| r.map(Body::HookRan)),
        task::STARTED => task::decode_started(version, data).map(|r| r.map(Body::TaskStarted)),
        task::ENDED => task::decode_ended(version, data).map(|r| r.map(Body::TaskEnded)),
        task::CREATED => task::decode_created(version, data).map(|r| r.map(Body::ScheduleCreated)),
        task::DELETED => task::decode_deleted(version, data).map(|r| r.map(Body::ScheduleDeleted)),
        _ => None,
    };
    let body = match known {
        Some(Ok(body)) => body,
        Some(Err(source)) => {
            return Err(DecodeError::Payload {
                id: raw.id,
                kind: raw.kind.to_string(),
                version,
                source,
            });
        }
        None => Body::Unknown(Unknown {
            kind: raw.kind,
            v: raw.v,
            data: raw.data,
        }),
    };
    Ok(Event { header, body })
}

impl Event {
    /// Encodes the event back into its stored form: known payloads at their latest version,
    /// unknown ones exactly as they were read.
    pub fn to_raw(&self) -> Result<RawEvent, EncodeError> {
        let header = self.header;
        match &self.body {
            Body::ThreadStarted(payload) => encode(header, payload),
            Body::ThreadCompacted(payload) => encode(header, payload),
            Body::ThreadArchived(payload) => encode(header, payload),
            Body::ThreadUnarchived(payload) => encode(header, payload),
            Body::ThreadDeleted(payload) => encode(header, payload),
            Body::TurnStarted(payload) => encode(header, payload),
            Body::MessageAdded(payload) => encode(header, payload),
            Body::ToolCallRequested(payload) => encode(header, payload),
            Body::ToolCallCompleted(payload) => encode(header, payload),
            Body::ModelResponded(payload) => encode(header, payload),
            Body::TurnEnded(payload) => encode(header, payload),
            Body::ApprovalRequested(payload) => encode(header, payload),
            Body::ApprovalDecided(payload) => encode(header, payload),
            Body::GoalSet(payload) => encode(header, payload),
            Body::GoalEnded(payload) => encode(header, payload),
            Body::GoalResumed(payload) => encode(header, payload),
            Body::WorkspaceBound(payload) => encode(header, payload),
            Body::WorkspaceRemoved(payload) => encode(header, payload),
            Body::HookRan(payload) => encode(header, payload),
            Body::TaskStarted(payload) => encode(header, payload),
            Body::TaskEnded(payload) => encode(header, payload),
            Body::ScheduleCreated(payload) => encode(header, payload),
            Body::ScheduleDeleted(payload) => encode(header, payload),
            Body::Unknown(unknown) => Ok(RawEvent {
                glave: ENVELOPE_VERSION,
                id: header.id,
                thread: header.thread,
                seq: header.seq,
                at: header.at,
                kind: unknown.kind.clone(),
                v: unknown.v,
                data: unknown.data.clone(),
            }),
        }
    }
}

/// Parses a payload's bytes as one specific version of one type.
pub(crate) fn parse<T: DeserializeOwned>(data: &RawValue) -> serde_json::Result<T> {
    serde_json::from_str(data.get())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::{At, Seq};
    use crate::id::{EventId, ThreadId};
    use crate::payload::{Outcome, Producer};

    fn header() -> Header {
        let at: jiff::Timestamp = "2026-10-07T21:14:03.512Z".parse().unwrap();
        Header {
            id: EventId::new(at, [1; 10]).unwrap(),
            thread: ThreadId::new(at, [2; 10]).unwrap(),
            seq: Seq::new(7).unwrap(),
            at: At::from(at),
        }
    }

    fn line(kind: &str, v: u32, data: &str) -> String {
        let raw = encode(
            header(),
            &TurnEnded {
                outcome: Outcome::Completed,
            },
        )
        .unwrap();
        raw.to_json().unwrap().replace(
            r#""type":"turn.ended","v":1,"data":{"outcome":"completed"}"#,
            &format!(r#""type":"{kind}","v":{v},"data":{data}"#),
        )
    }

    #[test]
    fn encode_then_decode_round_trips() {
        let payload = ThreadStarted {
            producer: Producer {
                name: "glave".into(),
                version: "0.0.0".into(),
            },
            fork: None,
            parent: None,
            for_: None,
            agent: None,
            origin: crate::payload::Origin::of(crate::payload::OriginKind::Prompt),
        };
        let raw = encode(header(), &payload).unwrap();
        assert_eq!(raw.kind.as_str(), "thread.started");
        let event = decode(raw).unwrap();
        assert_eq!(event.header, header());
        assert!(matches!(event.body, Body::ThreadStarted(ref p) if *p == payload));
    }

    #[test]
    fn unknown_types_pass_through_byte_for_byte() {
        let data = r#"{"anything":[1, 2,  3],"z":"last","a":"first"}"#;
        let raw = RawEvent::from_json(&line("future.thing", 3, data)).unwrap();
        let event = decode(raw).unwrap();
        let Body::Unknown(ref unknown) = event.body else {
            panic!("expected an unknown event");
        };
        assert_eq!(unknown.kind.as_str(), "future.thing");
        assert_eq!(unknown.data.get(), data);
        assert_eq!(event.to_raw().unwrap().data.get(), data);
    }

    #[test]
    fn newer_versions_of_known_types_pass_through() {
        let raw =
            RawEvent::from_json(&line("turn.ended", 2, r#"{"outcome":"done","why":"x"}"#)).unwrap();
        assert!(matches!(decode(raw).unwrap().body, Body::Unknown(_)));
    }

    #[test]
    fn unknown_envelope_versions_are_rejected() {
        let mut raw = encode(
            header(),
            &TurnEnded {
                outcome: Outcome::Failed,
            },
        )
        .unwrap();
        raw.glave = 2;
        assert!(matches!(
            decode(raw),
            Err(DecodeError::UnsupportedEnvelope { found: 2, .. })
        ));
    }

    #[test]
    fn payloads_that_break_their_schema_are_errors() {
        for data in [
            r#"{"outcome":"done"}"#,
            r#"{"outcome":"completed","extra":1}"#,
            "{}",
        ] {
            let raw = RawEvent::from_json(&line("turn.ended", 1, data)).unwrap();
            assert!(
                matches!(decode(raw), Err(DecodeError::Payload { .. })),
                "{data}"
            );
        }
    }

    #[test]
    fn every_payload_declares_a_valid_type_and_version() {
        for (kind, version) in KNOWN_VERSIONS {
            assert!(TypeName::new(*kind).is_ok(), "{kind}");
            assert!(PayloadVersion::new(*version).is_ok(), "{kind} v{version}");
        }
    }
}
