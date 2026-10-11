//! Running the hooks at one point of a turn: each gets its input as JSON on standard input,
//! runs on the workspace's computer (in its sandbox, when there is one), and is recorded as
//! `hook.ran`. They run one after another, and the first that blocks ends the rest.

use std::sync::{Arc, Mutex};

use glave_event::{Arguments, CallId, HookPoint, HookRan, HookResult, Outcome, Part};
use glave_ports::{BlobStore, BoxFuture, Ending, EventLog, ModelProvider, Output, ToolOutput};
use serde_json::{Map, Value, json};

use super::{HOOK_TIMEOUT_SECS, Hook, point_name};
use crate::approval::Approver;
use crate::cancel::Cancel;
use crate::harness::{Harness, HarnessError, Thread};
use crate::tools::workspace::{Workspace, explain};

/// The most output kept from a hook, in bytes; the rest is dropped.
const MAX_OUTPUT: usize = 64 * 1024;

/// The most characters of a reason passed on to the model or the person.
const MAX_REASON: usize = 4_000;

/// The line that ends a hook's input; JSON on one line can never be it.
const END_OF_INPUT: &str = "GLAVE_HOOK_INPUT";

/// A tool call, as the tool hooks see it.
pub(crate) struct ToolHook<'a> {
    pub(crate) call_id: &'a CallId,
    pub(crate) tool: &'a str,
    pub(crate) arguments: &'a Arguments,
}

impl<'a> ToolHook<'a> {
    /// The model's `call` of the tool `tool`.
    pub(crate) fn of(call: &'a glave_ports::ToolCall, tool: &'a str) -> Self {
        Self {
            call_id: &call.call_id,
            tool,
            arguments: &call.arguments,
        }
    }
}

/// What one hook decided.
struct Ran {
    result: HookResult,
    exit_code: Option<i32>,
    reason: Option<String>,
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Runs the `PreToolUse` hooks for `call`: the reason, when one blocks it.
    pub(crate) async fn before_tool(
        &self,
        thread: &mut Thread,
        call: &ToolHook<'_>,
        cancel: &Cancel,
    ) -> Result<Option<String>, HarnessError> {
        let input = tool_input(call);
        let point = HookPoint::PreToolUse;
        self.run_hooks(thread, point, Some(call), input, cancel)
            .await
    }

    /// Runs the `PostToolUse` hooks for `call`. A hook that blocks has its reason added to
    /// what the model is shown; the call already ran.
    pub(crate) async fn after_tool(
        &self,
        thread: &mut Thread,
        call: &ToolHook<'_>,
        mut output: ToolOutput,
        cancel: &Cancel,
    ) -> Result<ToolOutput, HarnessError> {
        let mut input = tool_input(call);
        input.insert(
            "tool_response".into(),
            json!({ "output": text_of(&output.parts), "is_error": output.is_error }),
        );
        let point = HookPoint::PostToolUse;
        if let Some(reason) = self
            .run_hooks(thread, point, Some(call), input, cancel)
            .await?
        {
            output.parts.push(Part::Text {
                text: format!("[A hook that runs after {} says: {reason}]", call.tool),
            });
        }
        Ok(output)
    }

    /// Runs the `UserPromptSubmit` hooks: the reason, when one stops the prompt.
    pub(crate) async fn on_prompt(
        &self,
        thread: &mut Thread,
        prompt: &str,
        cancel: &Cancel,
    ) -> Result<Option<String>, HarnessError> {
        let mut input = Map::new();
        input.insert("prompt".into(), Value::String(prompt.to_owned()));
        let point = HookPoint::UserPromptSubmit;
        self.run_hooks(thread, point, None, input, cancel).await
    }

    /// Runs the `Stop` hooks once a turn has ended. They cannot keep it going: a goal does
    /// that, with a check.
    pub(crate) async fn on_stop(
        &self,
        thread: &mut Thread,
        outcome: Outcome,
        cancel: &Cancel,
    ) -> Result<(), HarnessError> {
        let mut input = Map::new();
        input.insert("outcome".into(), json!(outcome));
        self.run_hooks(thread, HookPoint::Stop, None, input, cancel)
            .await
            .map(|_| ())
    }

    /// Runs the hooks at `point` in order, recording each; the first block's reason stops
    /// the rest. With no hooks there, nothing runs and nothing is recorded.
    async fn run_hooks(
        &self,
        thread: &mut Thread,
        point: HookPoint,
        call: Option<&ToolHook<'_>>,
        mut input: Map<String, Value>,
        cancel: &Cancel,
    ) -> Result<Option<String>, HarnessError> {
        let hooks = self.hooks.at(point, call.map(|call| call.tool));
        if hooks.is_empty() {
            return Ok(None);
        }
        input.insert("hook_event_name".into(), point_name(point).into());
        input.insert("session_id".into(), thread.id.to_string().into());
        let workspace = Arc::clone(self.workspace_for(thread));
        let root = workspace.root().display().to_string();
        input.insert("cwd".into(), root.into());
        let mode = self.settings.approval.name();
        input.insert("permission_mode".into(), mode.into());
        let input = Value::Object(input).to_string();
        for hook in hooks {
            let ran = Self::run_hook(&workspace, hook, &input, cancel).await;
            let blocked = (ran.result == HookResult::Blocked)
                .then(|| ran.reason.clone())
                .flatten();
            let event = HookRan {
                point,
                command: hook.command.clone(),
                call_id: call.map(|call| call.call_id.clone()),
                result: ran.result,
                exit_code: ran.exit_code,
                reason: ran.reason,
            };
            self.record(thread, &event).await?;
            if blocked.is_some() {
                return Ok(blocked);
            }
        }
        Ok(None)
    }

    /// Runs one hook with `input` on its standard input, and reads its answer.
    async fn run_hook(workspace: &Workspace, hook: &Hook, input: &str, cancel: &Cancel) -> Ran {
        // The input goes in as a here-document; one line of JSON never ends it early.
        let script = format!(
            "{{\n{}\n}} <<'{END_OF_INPUT}'\n{input}\n{END_OF_INPUT}\n",
            hook.command
        );
        let timeout = hook.timeout_secs.unwrap_or(HOOK_TIMEOUT_SECS);
        let output = Kept::default();
        let finished = workspace.run(script, timeout, &output, cancel).await;
        let text = output.text();
        let ending = match finished {
            Ok(finished) => finished.ending,
            Err(error) => return failed(None, format!("it could not run: {}", explain(&error))),
        };
        match ending {
            Ending::Exited { code: 0 } => decided(hook.point, &text),
            Ending::Exited { code: 2 } if hook.point == HookPoint::Stop => failed(
                Some(2),
                "a Stop hook cannot keep the turn going; its exit code 2 was ignored".to_owned(),
            ),
            Ending::Exited { code: 2 } => Ran {
                result: HookResult::Blocked,
                exit_code: Some(2),
                reason: Some(reason_or_default(&text)),
            },
            Ending::Exited { code } => failed(
                Some(code),
                format!("it exited with code {code}: {}", clip(text.trim())),
            ),
            Ending::Signalled => failed(None, "it was stopped by a signal".to_owned()),
            Ending::TimedOut { after_secs } => {
                failed(None, format!("it timed out after {after_secs} s"))
            }
            Ending::Cancelled => failed(None, "the turn was cancelled".to_owned()),
        }
    }
}

/// A hook that exited 0 passes, unless its output is a JSON object that blocks, as Claude
/// Code and Codex read it: `"decision": "block"`, or for `PreToolUse` a
/// `hookSpecificOutput.permissionDecision` of `"deny"`. Allowing fields are ignored: a hook
/// never lets anything through that would otherwise ask.
fn decided(point: HookPoint, text: &str) -> Ran {
    let passed = Ran {
        result: HookResult::Passed,
        exit_code: Some(0),
        reason: None,
    };
    let Ok(Value::Object(answer)) = serde_json::from_str::<Value>(text.trim()) else {
        return passed;
    };
    let field = |object: &Map<String, Value>, key: &str| {
        object.get(key).and_then(Value::as_str).map(str::to_owned)
    };
    let specific = answer.get("hookSpecificOutput").and_then(Value::as_object);
    let denied = specific
        .filter(|_| point == HookPoint::PreToolUse)
        .filter(|output| field(output, "permissionDecision").as_deref() == Some("deny"))
        .map(|output| field(output, "permissionDecisionReason").unwrap_or_default());
    let blocked = (field(&answer, "decision").as_deref() == Some("block"))
        .then(|| field(&answer, "reason").unwrap_or_default());
    match denied.or(blocked) {
        Some(_) if point == HookPoint::Stop => failed(
            Some(0),
            "a Stop hook cannot keep the turn going; its block was ignored".to_owned(),
        ),
        Some(reason) => Ran {
            result: HookResult::Blocked,
            exit_code: Some(0),
            reason: Some(reason_or_default(&reason)),
        },
        None => passed,
    }
}

fn failed(exit_code: Option<i32>, why: String) -> Ran {
    Ran {
        result: HookResult::Failed,
        exit_code,
        reason: Some(why),
    }
}

/// A block's reason, trimmed and kept short, or a plain one when the hook gave none.
fn reason_or_default(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        "a hook blocked this without saying why".to_owned()
    } else {
        clip(text)
    }
}

/// At most [`MAX_REASON`] characters of `text`.
fn clip(text: &str) -> String {
    match text.char_indices().nth(MAX_REASON) {
        Some((at, _)) => format!("{}…", text.get(..at).unwrap_or(text)),
        None => text.to_owned(),
    }
}

/// The input every tool hook gets: the tool, its arguments and the call's id.
fn tool_input(call: &ToolHook<'_>) -> Map<String, Value> {
    let arguments = match call.arguments {
        Arguments::Json(map) => Value::Object(map.clone()),
        Arguments::Malformed(raw) => Value::String(raw.clone()),
    };
    let mut input = Map::new();
    input.insert("tool_name".into(), call.tool.into());
    input.insert("tool_input".into(), arguments);
    input.insert("tool_use_id".into(), call.call_id.as_str().into());
    input
}

/// The text parts of a tool's result, joined.
fn text_of(parts: &[Part]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            Part::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A hook's output, up to [`MAX_OUTPUT`] bytes.
#[derive(Default)]
struct Kept(Mutex<String>);

impl Kept {
    fn text(&self) -> String {
        self.0.lock().map(|text| text.clone()).unwrap_or_default()
    }
}

impl Output for Kept {
    fn write<'a>(&'a self, text: &'a str) -> BoxFuture<'a, ()> {
        if let Ok(mut kept) = self.0.lock() {
            let room = MAX_OUTPUT.saturating_sub(kept.len());
            let mut end = room.min(text.len());
            while !text.is_char_boundary(end) {
                end = end.saturating_sub(1);
            }
            kept.push_str(text.get(..end).unwrap_or_default());
        }
        Box::pin(std::future::ready(()))
    }
}

#[cfg(test)]
pub(crate) fn decided_for_tests(point: HookPoint, text: &str) -> (HookResult, Option<String>) {
    let ran = decided(point, text);
    (ran.result, ran.reason)
}
