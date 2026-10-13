//! The payload of every event type at its latest version. Each module also holds its type's
//! decoder, which upgrades older versions on read.

pub(crate) mod approval;
pub(crate) mod chosen;
pub(crate) mod gateway;
pub(crate) mod goal;
pub(crate) mod hook;
pub(crate) mod memory;
pub(crate) mod message;
pub(crate) mod model;
pub(crate) mod part;
pub(crate) mod principal;
pub(crate) mod provider;
pub(crate) mod standing;
pub(crate) mod task;
pub(crate) mod thread;
pub(crate) mod tool;
pub(crate) mod turn;
pub(crate) mod workspace;

pub use approval::{ApprovalDecided, ApprovalRequested, Verdict};
pub use chosen::{ApprovalMode, ApprovalModeChosen, ModelChosen};
pub use gateway::GatewayToolCalled;
pub use goal::{GoalEnded, GoalReason, GoalResumed, GoalSet};
pub use hook::{HookPoint, HookRan, HookResult};
pub use memory::{
    MemoryEdited, MemoryForgotten, MemoryRestored, MemorySaved, MemoryScope, MemorySource,
    MemoryTidied, sealed_under_memory,
};
pub use message::{MessageAdded, Role};
pub use model::{Cost, ModelRef, ModelResponded, StopReason, TokenCounts, Usage};
pub use part::{Opaque, Part};
pub use principal::{Principal, PrincipalKind};
pub use provider::{ProviderConnected, ProviderKind, ProviderRemoved, sealed_under_provider};
pub use standing::{ThreadArchived, ThreadDeleted, ThreadRenamed, ThreadUnarchived};
pub use task::{Fired, ScheduleCreated, ScheduleDeleted, TaskEnded, TaskStarted, When};
pub use thread::{
    ForkPoint, Origin, OriginKind, ParentCall, Producer, ThreadCompacted, ThreadStarted,
};
pub use tool::{Arguments, ToolCallCompleted, ToolCallRequested};
pub use turn::{Auth, AuthMethod, Outcome, TraceId, TurnEnded, TurnStarted};
pub use workspace::{
    ComputerKind, ComputerRef, Opened, RemovedReason, WorkspaceBound, WorkspaceRemoved,
};
