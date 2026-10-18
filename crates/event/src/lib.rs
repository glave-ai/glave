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
mod secret;

pub use blob::{BlobRef, Sha256};
pub use codec::{Body, Event, KNOWN_VERSIONS, Payload, Unknown, decode, encode};
pub use envelope::{At, Header, PayloadVersion, RawEvent, Seq, TypeName};
pub use error::{DecodeError, EncodeError, InvalidValue};
pub use id::{
    CallId, CredentialId, EventId, MemoryId, PrincipalId, ProviderId, ScheduleId, ThreadId,
};
pub use money::{Currency, Money};
pub use payload::{
    ApprovalDecided, ApprovalMode, ApprovalModeChosen, ApprovalRequested, Arguments, Auth,
    AuthMethod, Cost, ForkPoint, GatewayToolCalled, GoalEnded, GoalReason, GoalResumed, GoalSet,
    HookPoint, HookRan, HookResult, MemoryEdited, MemoryForgotten, MemoryRestored, MemorySaved,
    MemoryScope, MemorySource, MemoryTidied, MessageAdded, ModelChosen, ModelRef, ModelResponded,
    Opaque, Origin, OriginKind, Outcome, OutsideSandbox, ParentCall, Part, Principal,
    PrincipalKind, Producer, Role, StopReason, ThreadArchived, ThreadCompacted, ThreadDeleted,
    ThreadRenamed, ThreadStarted, ThreadUnarchived, TokenCounts, ToolCallCompleted,
    ToolCallRequested, TraceId, TurnEnded, TurnStarted, Usage, Verdict,
};
pub use payload::{Checkout, CommandEnded, CommandEnding, CommandStarted, FolderSource};
pub use payload::{
    ComputerKind, ComputerRef, Fired, Opened, RemovedReason, ScheduleCreated, ScheduleDeleted,
    TaskEnded, TaskStarted, When, WorkspaceBound, WorkspaceRemoved, sealed_under_memory,
};
pub use payload::{
    CredentialConnected, CredentialRemoved, Forge, ProviderConnected, ProviderKind,
    ProviderRemoved, Renewal, SpaceFolderSet, SpaceRemoved, sealed_under_account,
};
pub use secret::{Secret, serialize_exposed};
