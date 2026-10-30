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
    CallId, CredentialId, EventId, InviteId, MemoryId, PrincipalId, ProviderId, ScheduleId,
    ThreadId,
};
pub use money::{Currency, Decimal, Money};
pub use payload::{
    AcmeRegistered, AwakeSet, CertificateIssued, Filters, FiltersSet, Grouping, GroupingSet, Hides,
    InstructionsFile, InstructionsRead, LanguageSet, MailAsked, MaxPrice, MemorySet, Serve,
    SignupFinished, SignupStarted, Sort,
};
pub use payload::{
    ApprovalDecided, ApprovalRequested, Arguments, Auth, AuthMethod, Cost, EffortChosen, ForkPoint,
    GatewayToolCalled, GoalEnded, GoalReason, GoalResumed, GoalSet, HookPoint, HookRan, HookResult,
    Lifted, MemoryEdited, MemoryForgotten, MemoryRestored, MemorySaved, MemoryScope, MemorySource,
    MemoryTidied, MessageAdded, ModelChosen, ModelRef, ModelResponded, Opaque, Origin, OriginKind,
    Outcome, OutsideSandbox, ParentCall, Part, Permissions, PermissionsChosen, Principal,
    PrincipalKind, Producer, Role, StopReason, Thought, ThreadArchived, ThreadCompacted,
    ThreadDeleted, ThreadRenamed, ThreadStarted, ThreadUnarchived, TokenCounts, ToolCallCompleted,
    ToolCallRequested, ToolKind, TraceId, TurnResult, TurnStarted, Usage, Verdict, WebPage,
};
pub use payload::{BrowserSignedIn, BrowserSignedOut, BrowserUsed, Decision, TrustDecided};
pub use payload::{
    Catalog, CatalogEndpoint, CatalogModel, CatalogProvider, CatalogRefreshed, Precision,
    Reasoning, TokenPrice,
};
pub use payload::{Caution, Phrase, Risk, Settled, Worded, sentence};
pub use payload::{Checkout, CommandEnded, CommandEnding, CommandStarted, FolderSource};
pub use payload::{
    ComputerKind, ComputerRef, Fired, Opened, RemovedReason, ScheduleCreated, ScheduleDeleted,
    TaskEnded, TaskStarted, When, WorkspaceBound, WorkspaceRemoved, sealed_under_memory,
};
pub use payload::{ConnectionConnected, ConnectionRemoved, SignInClient};
pub use payload::{CostKind, CreditsAdded, CreditsCharged, CreditsLimitSet};
pub use payload::{
    CredentialConnected, CredentialRemoved, Forge, ProjectFolderSet, ProjectRemoved,
    ProviderConnected, ProviderKind, ProviderRemoved, Renewal, sealed_under_account,
};
pub use payload::{
    InviteAccepted, InviteCreated, InviteRevoked, OrgRole, PersonJoined, PersonRemoved, Profile,
    RoleSet,
};
pub use payload::{PasskeyAdded, PasskeyRemoved, PasskeyUsed};
pub use payload::{PlanApproved, PlanStatus, PlanStep, PlanUpdated};
pub use secret::{Secret, serialize_exposed};
