//! The payload of every event type at its latest version. Each module also holds its type's
//! decoder, which upgrades older versions on read.

pub(crate) mod approval;
pub(crate) mod goal;
pub(crate) mod hook;
pub(crate) mod message;
pub(crate) mod model;
pub(crate) mod part;
pub(crate) mod principal;
pub(crate) mod thread;
pub(crate) mod tool;
pub(crate) mod turn;

pub use approval::{ApprovalDecided, ApprovalRequested, Verdict};
pub use goal::{GoalEnded, GoalReason, GoalSet};
pub use hook::{HookPoint, HookRan, HookResult};
pub use message::{MessageAdded, Role};
pub use model::{Cost, ModelRef, ModelResponded, StopReason, TokenCounts, Usage};
pub use part::{Opaque, Part};
pub use principal::{Principal, PrincipalKind};
pub use thread::{
    ForkPoint, Origin, OriginKind, ParentCall, Producer, ThreadCompacted, ThreadStarted,
};
pub use tool::{Arguments, ToolCallCompleted, ToolCallRequested};
pub use turn::{Auth, AuthMethod, Outcome, TraceId, TurnEnded, TurnStarted};
