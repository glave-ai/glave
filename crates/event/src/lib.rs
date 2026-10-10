//! glave's event format: the envelope every event shares, the payload types, and the
//! decoder that upgrades old events as it reads them.
//!
//! The contract is `specs/events.md`. Writers build a [`Header`] and a payload, then
//! [`encode`] them into a [`RawEvent`], the exact form that is stored and sent. Readers
//! parse a [`RawEvent`] and [`decode`] it into an [`Event`], whose [`Body`] holds the
//! latest version of every known payload and keeps unknown events untouched.

mod blob;
mod codec;
mod envelope;
mod error;
mod id;
mod money;
mod payload;

pub use blob::{BlobRef, Sha256};
pub use codec::{Body, Event, KNOWN_VERSIONS, Payload, Unknown, decode, encode};
pub use envelope::{At, ENVELOPE_VERSION, Header, PayloadVersion, RawEvent, Seq, TypeName};
pub use error::{DecodeError, EncodeError, InvalidValue};
pub use id::{CallId, EventId, PrincipalId, ThreadId};
pub use money::{Currency, Money};
pub use payload::{
    ApprovalDecided, ApprovalRequested, Arguments, Auth, AuthMethod, Cost, ForkPoint, GoalEnded,
    GoalReason, GoalResumed, GoalSet, HookPoint, HookRan, HookResult, MessageAdded, ModelRef,
    ModelResponded, Opaque, Origin, OriginKind, Outcome, ParentCall, Part, Principal,
    PrincipalKind, Producer, Role, StopReason, ThreadCompacted, ThreadStarted, TokenCounts,
    ToolCallCompleted, ToolCallRequested, TraceId, TurnEnded, TurnStarted, Usage, Verdict,
};
