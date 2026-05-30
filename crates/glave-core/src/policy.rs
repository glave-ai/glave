//! Mediation — server-side governance of every tool call. Mode-aware: the eval
//! runner uses `AutonomousPolicy` (zero human-in-the-loop — `Ask` is treated as
//! `Deny` by the caller). A future product runtime can implement `Policy` to
//! resolve `Ask` into a real human prompt, or auto-accept edits, etc.

use crate::tools::ToolCall;

pub enum Decision {
    Allow,
    Deny(String),
    /// Defer to a human (product modes). The autonomous eval runner treats this
    /// as `Deny`, so the eval never blocks on a human.
    Ask(String),
}

pub trait Policy {
    fn decide(&self, call: &ToolCall) -> Decision;
}

/// Autonomous (eval) policy: enforces the task's editable surface on edits;
/// allows reads, bash, and tests (the container is the security boundary). Never
/// asks a human — zero human-in-the-loop by construction.
pub struct AutonomousPolicy {
    editable: Vec<glob::Pattern>,
}

impl AutonomousPolicy {
    pub fn new(editable_globs: &[String]) -> Self {
        let editable = editable_globs
            .iter()
            .filter_map(|g| glob::Pattern::new(g).ok())
            .collect();
        Self { editable }
    }

    fn path_editable(&self, path: &str) -> bool {
        if self.editable.is_empty() {
            return true;
        }
        let base = path.rsplit('/').next().unwrap_or(path);
        self.editable
            .iter()
            .any(|p| p.matches(path) || p.matches(base))
    }
}

impl Policy for AutonomousPolicy {
    fn decide(&self, call: &ToolCall) -> Decision {
        match call {
            ToolCall::StrReplace { path, .. } | ToolCall::Create { path, .. } => {
                if self.path_editable(path) {
                    Decision::Allow
                } else {
                    Decision::Deny(format!("{path} is outside the editable surface"))
                }
            }
            ToolCall::Read { .. }
            | ToolCall::Bash { .. }
            | ToolCall::Grep { .. }
            | ToolCall::Glob { .. }
            | ToolCall::RunTests
            | ToolCall::Subagent { .. }
            | ToolCall::Submit => Decision::Allow,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create(path: &str) -> ToolCall {
        ToolCall::Create {
            path: path.to_string(),
            content: String::new(),
        }
    }

    #[test]
    fn editable_surface_gates_edits() {
        let p = AutonomousPolicy::new(&["**/*.go".to_string()]);
        assert!(matches!(p.decide(&create("config.go")), Decision::Allow));
        assert!(matches!(p.decide(&create("order/total.go")), Decision::Allow));
        assert!(matches!(p.decide(&create("secret.py")), Decision::Deny(_)));
    }

    #[test]
    fn empty_surface_allows_all() {
        let p = AutonomousPolicy::new(&[]);
        assert!(matches!(p.decide(&create("anything.rs")), Decision::Allow));
    }

    #[test]
    fn bash_always_allowed() {
        let p = AutonomousPolicy::new(&["**/*.go".to_string()]);
        let bash = ToolCall::Bash {
            cmd: "rm x".to_string(),
        };
        assert!(matches!(p.decide(&bash), Decision::Allow));
    }
}
