//! Hooks as configured: commands the person, or a project they trust, sets to run at four
//! points of a turn (`PreToolUse`, `PostToolUse`, `UserPromptSubmit`, `Stop`), each checked
//! before it is used. The harness runs them; config and policy files fill them.

use glave_event::HookPoint;
use regex::Regex;
use thiserror::Error;

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

/// Why a hook cannot be used as configured. A hook is named by its point and where it comes
/// among that point's, never by its command, which may hold a secret (a token in a `curl`).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HookError {
    #[error(
        "{point} hook number {nth} has matcher {matcher:?}, which is not a regular expression: {why}"
    )]
    Matcher {
        point: &'static str,
        nth: usize,
        matcher: String,
        why: String,
    },
    #[error("{point} hook number {nth} has a matcher, but {point} hooks do not run for a tool")]
    NoTool { point: &'static str, nth: usize },
    #[error("{point} hook number {nth} has an empty command")]
    Empty { point: &'static str, nth: usize },
}

/// The hooks a harness runs, in the order configured.
#[derive(Debug, Clone, Default)]
pub struct Hooks {
    hooks: Vec<(Hook, Option<Regex>)>,
}

impl Hooks {
    /// `hooks`, each checked: a command, and a matcher only where there is a tool to match.
    pub fn new(hooks: Vec<Hook>) -> Result<Self, HookError> {
        let mut checked: Vec<(Hook, Option<Regex>)> = Vec::with_capacity(hooks.len());
        for hook in hooks {
            let before = checked.iter().filter(|(kept, _)| kept.point == hook.point);
            let regex = matcher(&hook, before.count() + 1)?;
            checked.push((hook, regex));
        }
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
    pub fn at(&self, point: HookPoint, tool: Option<&str>) -> Vec<&Hook> {
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

/// The compiled matcher of `hook`, the `nth` at its point, anchored to the whole tool name;
/// `None` matches all.
fn matcher(hook: &Hook, nth: usize) -> Result<Option<Regex>, HookError> {
    let point = point_name(hook.point);
    if hook.command.trim().is_empty() {
        return Err(HookError::Empty { point, nth });
    }
    let Some(pattern) = hook.matcher.as_deref().map(str::trim) else {
        return Ok(None);
    };
    if !for_tools(hook.point) {
        return Err(HookError::NoTool { point, nth });
    }
    if pattern.is_empty() || pattern == "*" {
        return Ok(None);
    }
    Regex::new(&format!("^(?:{pattern})$"))
        .map(Some)
        .map_err(|error| HookError::Matcher {
            point,
            nth,
            matcher: pattern.to_owned(),
            why: error.to_string(),
        })
}

#[cfg(test)]
mod tests;
