//! Hooks: commands the person, or a project they trust, configures to run at four points of a
//! turn, as Claude Code and Codex run them: before a tool call (`PreToolUse`), after it
//! (`PostToolUse`), when the person sends a prompt (`UserPromptSubmit`) and when a turn ends
//! (`Stop`). A hook gets JSON on its standard input and answers with its exit code, or with
//! JSON on exit 0. A hook can stop a tool call or a prompt, or tell the model something after
//! a tool ran; it can never let anything through that would otherwise ask (`specs/approvals.md`).

mod run;
#[cfg(test)]
mod tests;

use regex::Regex;
use thiserror::Error;

pub use glave_event::HookPoint;
pub(crate) use run::ToolHook;

/// How long a hook may run, in seconds, when it does not say: the bash tool's own limit, and
/// Claude Code's and Codex's default.
pub const HOOK_TIMEOUT_SECS: u64 = 600;

/// One hook as configured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hook {
    pub point: HookPoint,
    /// A bash command, run at the workspace root on the workspace's computer.
    pub command: String,
    /// For the tool points: a regular expression the whole tool name must match. `None`, or
    /// `*`, matches every tool.
    pub matcher: Option<String>,
    /// Seconds before it is stopped; [`HOOK_TIMEOUT_SECS`] when `None`.
    pub timeout_secs: Option<u64>,
}

/// Why a hook cannot be used as configured.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HookError {
    #[error(
        "the {point} hook {command:?} has matcher {matcher:?}, which is not a regular expression: {why}"
    )]
    Matcher {
        point: &'static str,
        command: String,
        matcher: String,
        why: String,
    },
    #[error("the {point} hook {command:?} has a matcher, but {point} hooks do not run for a tool")]
    NoTool {
        point: &'static str,
        command: String,
    },
    #[error("the {point} hook has an empty command")]
    Empty { point: &'static str },
}

/// The hooks a harness runs, in the order configured.
#[derive(Debug, Clone, Default)]
pub struct Hooks {
    hooks: Vec<(Hook, Option<Regex>)>,
}

impl Hooks {
    /// `hooks`, each checked: a command, and a matcher only where there is a tool to match.
    pub fn new(hooks: Vec<Hook>) -> Result<Self, HookError> {
        let checked = hooks
            .into_iter()
            .map(|hook| {
                let regex = matcher(&hook)?;
                Ok((hook, regex))
            })
            .collect::<Result<_, HookError>>()?;
        Ok(Self { hooks: checked })
    }

    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    /// Every hook, in order.
    pub fn all(&self) -> impl Iterator<Item = &Hook> {
        self.hooks.iter().map(|(hook, _)| hook)
    }

    /// The hooks that run at `point`, for `tool` at the tool points.
    pub(crate) fn at(&self, point: HookPoint, tool: Option<&str>) -> Vec<&Hook> {
        self.hooks
            .iter()
            .filter(|(hook, _)| hook.point == point)
            .filter(|(_, regex)| match (regex, tool) {
                (Some(regex), Some(tool)) => regex.is_match(tool),
                _ => true,
            })
            .map(|(hook, _)| hook)
            .collect()
    }
}

/// The name a point has in configuration and in a hook's input, as Claude Code and Codex
/// name it.
pub fn point_name(point: HookPoint) -> &'static str {
    match point {
        HookPoint::PreToolUse => "PreToolUse",
        HookPoint::PostToolUse => "PostToolUse",
        HookPoint::UserPromptSubmit => "UserPromptSubmit",
        HookPoint::Stop => "Stop",
    }
}

/// Whether hooks at `point` run for a tool, and so may have a matcher.
fn for_tools(point: HookPoint) -> bool {
    matches!(point, HookPoint::PreToolUse | HookPoint::PostToolUse)
}

/// The compiled matcher of `hook`, anchored to the whole tool name; `None` matches all.
fn matcher(hook: &Hook) -> Result<Option<Regex>, HookError> {
    let point = point_name(hook.point);
    if hook.command.trim().is_empty() {
        return Err(HookError::Empty { point });
    }
    let Some(pattern) = hook.matcher.as_deref().map(str::trim) else {
        return Ok(None);
    };
    if !for_tools(hook.point) {
        return Err(HookError::NoTool {
            point,
            command: hook.command.clone(),
        });
    }
    if pattern.is_empty() || pattern == "*" {
        return Ok(None);
    }
    Regex::new(&format!("^(?:{pattern})$"))
        .map(Some)
        .map_err(|error| HookError::Matcher {
            point,
            command: hook.command.clone(),
            matcher: pattern.to_owned(),
            why: error.to_string(),
        })
}
