//! How much glave asks before acting, as the API names it: the server's mode, the modes a
//! thread may choose, and each thread's own.

use serde::{Deserialize, Serialize};

/// How much glave asks before acting (`specs/approvals.md`, "Modes").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    /// The cautious mode: ask before every change and every command.
    Ask,
    /// Changes and commands go through; risky ones still ask.
    Auto,
}

impl ApprovalMode {
    /// The mode's name, as the API writes it.
    pub fn name(self) -> &'static str {
        glave_harness::ApprovalMode::from(self).name()
    }
}

impl From<glave_harness::ApprovalMode> for ApprovalMode {
    fn from(mode: glave_harness::ApprovalMode) -> Self {
        match mode {
            glave_harness::ApprovalMode::Ask => Self::Ask,
            glave_harness::ApprovalMode::Auto => Self::Auto,
        }
    }
}

impl From<ApprovalMode> for glave_harness::ApprovalMode {
    fn from(mode: ApprovalMode) -> Self {
        match mode {
            ApprovalMode::Ask => Self::Ask,
            ApprovalMode::Auto => Self::Auto,
        }
    }
}
