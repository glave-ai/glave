//! The payload of every event type at its latest version. Each module also holds its type's
//! decoder, which upgrades older versions on read.

pub(crate) mod approval;
pub(crate) mod awake;
pub(crate) mod browser;
pub(crate) mod catalog;
pub(crate) mod certificate;
pub(crate) mod chosen;
pub(crate) mod command;
pub(crate) mod connection;
pub(crate) mod credential;
pub(crate) mod gateway;
pub(crate) mod goal;
pub(crate) mod grouping;
pub(crate) mod hook;
pub(crate) mod language;
pub(crate) mod memory;
pub(crate) mod message;
pub(crate) mod model;
pub(crate) mod part;
pub(crate) mod passkey;
pub(crate) mod plan;
pub(crate) mod principal;
pub(crate) mod project;
pub(crate) mod provider;
pub(crate) mod standing;
pub(crate) mod task;
pub(crate) mod thread;
pub(crate) mod tool;
pub(crate) mod trust;
pub(crate) mod turn;
pub(crate) mod workspace;

pub use approval::{ApprovalDecided, ApprovalRequested, Lifted, OutsideSandbox, Verdict};
pub use awake::AwakeSet;
pub use browser::{BrowserSignedIn, BrowserSignedOut, BrowserUsed};
pub use catalog::{Catalog, CatalogModel, CatalogRefreshed, Reasoning, TokenPrice};
pub use certificate::{AcmeRegistered, CertificateIssued};
pub use chosen::{EffortChosen, ModelChosen, Permissions, PermissionsChosen};
pub use command::{CommandEnded, CommandEnding, CommandStarted};
pub use connection::{ConnectionConnected, ConnectionRemoved, SignInClient};
pub use credential::{CredentialConnected, CredentialRemoved, Forge, Renewal};
pub use gateway::GatewayToolCalled;
pub use goal::{GoalEnded, GoalReason, GoalResumed, GoalSet};
pub use grouping::{Grouping, GroupingSet};
pub use hook::{HookPoint, HookRan, HookResult};
pub use language::LanguageSet;
pub use memory::{
    MemoryEdited, MemoryForgotten, MemoryRestored, MemorySaved, MemoryScope, MemorySource,
    MemoryTidied, sealed_under_memory,
};
pub use message::{MessageAdded, Role};
pub use model::{Cost, ModelRef, ModelResponded, StopReason, TokenCounts, Usage};
pub use part::{Opaque, Part};
pub use passkey::{PasskeyAdded, PasskeyRemoved, PasskeyUsed};
pub use plan::{PlanApproved, PlanStatus, PlanStep, PlanUpdated};
pub use principal::{Principal, PrincipalKind};
pub use project::{Checkout, FolderSource, ProjectFolderSet, ProjectRemoved};
pub use provider::{ProviderConnected, ProviderKind, ProviderRemoved, sealed_under_account};
pub use standing::{ThreadArchived, ThreadDeleted, ThreadRenamed, ThreadUnarchived};
pub use task::{Fired, ScheduleCreated, ScheduleDeleted, TaskEnded, TaskStarted, When};
pub use thread::{
    ForkPoint, Origin, OriginKind, ParentCall, Producer, ThreadCompacted, ThreadStarted,
};
pub use tool::{Arguments, ToolCallCompleted, ToolCallRequested, WebPage};
pub use trust::{Decision, TrustDecided};
pub use turn::{Auth, AuthMethod, Outcome, TraceId, TurnEnded, TurnStarted};
pub use workspace::{
    ComputerKind, ComputerRef, Opened, RemovedReason, WorkspaceBound, WorkspaceRemoved,
};
