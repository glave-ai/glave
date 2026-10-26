//! glave's gateway, in-process: the single door out for model calls. It is a
//! [`ModelProvider`](glave_ports::ModelProvider) wrapping others, so the harness never knows it
//! is there.
//!
//! - **Routes** map each purpose (`main`, `judge`, `summary`, `subagent`) to an ordered list of
//!   targets; a purpose with no route of its own uses `main`'s.
//! - **Fallback** is capability-checked (a target that cannot honour the request is skipped,
//!   never sent a degraded one), sticky to the provider whose opaque reasoning the conversation
//!   holds, and moves on only after failures that may pass.
//! - **Budgets**: a signed-in caller's reserves a call's worst case before it is sent and
//!   settles to what it cost, so calls running side by side never overshoot it.
//! - **Metering**: every answer says the route and target that served it, for `model.responded`.
//!
//! The contract is `specs/gateway.md`.

mod budget;
mod estimate;
mod gateway;
mod plan;
mod price;
mod routes;
mod target;
mod telemetry;

pub use budget::{BudgetKey, Budgets, OverBudget};
pub use gateway::Gateway;
pub use price::Price;
pub use routes::{RouteError, Routes};
pub use target::{Capabilities, Model, Target};
