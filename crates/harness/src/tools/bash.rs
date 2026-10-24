//! `bash`: run one command in a fresh shell at the workspace root, on the workspace's
//! computer, keeping the start and the end of what it prints. One still running at its timeout
//! keeps running in the background, as one started there with `background` does
//! (`background.rs`). A call may ask to run outside the sandbox, or in it with the network; the
//! harness asks the person before it lets one through (`sandbox::outside`).

use std::sync::Arc;
use std::time::Duration;

use glave_event::{Arguments, Lifted, OutsideSandbox};
use glave_ports::Cancellation;
use glave_ports::computer::{ComputerError, Ending as Exit, OnTimeout, Sandbox};
use serde::Deserialize;

use super::background::{moved, start};
use super::captured::Capture;
use super::workspace::{Workspace, explain};
use super::{Tool, ToolOutcome, parse};
use crate::background::Commands;
use crate::sandbox::blocked_note;

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_TIMEOUT_SECS: u64 = 600;
/// The most of the model's reason for running outside the sandbox the person is shown.
const MOST_REASON_CHARS: usize = 500;

#[derive(Debug, Deserialize)]
pub(crate) struct BashArgs {
    command: String,
    #[serde(default)]
    timeout_secs: Option<u64>,
    /// Start it in the background and answer at once.
    #[serde(default)]
    background: bool,
    /// Run outside the sandbox, where there is one.
    #[serde(default)]
    outside_sandbox: bool,
    /// Run with the network, in the sandbox otherwise, where the sandbox has none.
    #[serde(default)]
    network: bool,
    /// Why, in the model's words, for the person.
    #[serde(default)]
    reason: Option<String>,
}

impl BashArgs {
    /// What of the sandbox the call asks to run without: all of it before only the network.
    fn lifted(&self) -> Option<Lifted> {
        match (self.outside_sandbox, self.network) {
            (true, _) => Some(Lifted::Sandbox),
            (false, true) => Some(Lifted::Network),
            (false, false) => None,
        }
    }
}

/// What of the sandbox a `bash` call asks to run without, with the model's reason, read as the
/// tool reads its arguments; `None` when it asks nothing of the kind.
pub(crate) fn outside_sandbox(arguments: &Arguments) -> Option<OutsideSandbox> {
    let args: BashArgs = parse(Tool::Bash, arguments).ok()?;
    let lifted = args.lifted()?;
    let reason = args
        .reason
        .map(|reason| reason.trim().chars().take(MOST_REASON_CHARS).collect())
        .filter(|reason: &String| !reason.is_empty());
    Some(OutsideSandbox { reason, lifted })
}

pub(crate) async fn bash(
    workspace: &Workspace,
    commands: &Commands,
    args: BashArgs,
    cancel: &dyn Cancellation,
) -> ToolOutcome {
    if args.command.trim().is_empty() {
        return ToolOutcome::error("The command is empty.");
    }
    let let_out = match let_out(workspace, &args).await {
        Ok(let_out) => let_out,
        Err(error) => return ToolOutcome::error(explain(&error)),
    };
    let workspace = let_out.as_ref().unwrap_or(workspace);
    if args.background {
        return start(workspace, commands, args.command, cancel).await;
    }
    let limit = args
        .timeout_secs
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .clamp(1, MAX_TIMEOUT_SECS);
    let captured = Capture::default();
    let script = args.command.clone();
    let timeout = (limit, OnTimeout::Background);
    let finished = match workspace.run(script, timeout, &captured, cancel).await {
        Ok(finished) => finished,
        Err(error) => return ToolOutcome::error(sentence(error.message())),
    };
    let ending = match finished.ending {
        Exit::Exited { code } => Ending::Exited(Some(code)),
        Exit::Signalled => Ending::Exited(None),
        Exit::Backgrounded {
            command,
            after_secs,
        } => {
            let computers = Arc::clone(workspace.computers());
            let id = workspace.info().id;
            let number = commands.add(command, id, computers, args.command, true);
            return moved(number, after_secs, &captured.render());
        }
        Exit::TimedOut { after_secs } | Exit::Idle { after_secs } => Ending::TimedOut(after_secs),
        Exit::Cancelled => Ending::Cancelled,
    };
    let output = captured.render();
    let seconds = Duration::from_millis(finished.millis).as_secs_f64();
    let footer = footer(&ending, seconds);
    let body = if output.is_empty() {
        "(no output)".to_owned()
    } else {
        output
    };
    let failed = !matches!(ending, Ending::Exited(Some(0)));
    let mut text = format!("{}\n{footer}", body.trim_end_matches('\n'));
    let exit = match ending {
        Ending::Exited(code) => code,
        Ending::TimedOut(_) | Ending::Cancelled => None,
    };
    if let Some(note) = workspace
        .sandbox()
        .and_then(|sandbox| blocked_note(sandbox, exit, &body))
    {
        text.push('\n');
        text.push_str(note);
    }
    if failed {
        ToolOutcome::error(text)
    } else {
        ToolOutcome::ok(text)
    }
}

/// The workspace this one command runs in instead of `workspace`, as the call was allowed
/// before it got here (`calls.rs`): outside its sandbox, or in it with the network where it has
/// none. `None` when it runs in the workspace's own.
async fn let_out(
    workspace: &Workspace,
    args: &BashArgs,
) -> Result<Option<Workspace>, ComputerError> {
    let (Some(lifted), Some(sandbox)) = (args.lifted(), workspace.sandbox()) else {
        return Ok(None);
    };
    let wider = match lifted {
        Lifted::Sandbox => None,
        Lifted::Network if sandbox.network => return Ok(None),
        Lifted::Network => Some(Sandbox {
            network: true,
            ..sandbox.clone()
        }),
    };
    workspace.within(wider.as_ref()).await.map(Some)
}

/// A computer's message as the start of a sentence: "could not start bash: …" reads
/// "Could not start bash: …".
fn sentence(message: &str) -> String {
    let mut chars = message.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// The line after a command's output: how it ended and how long it took.
fn footer(ending: &Ending, seconds: f64) -> String {
    match *ending {
        Ending::Exited(Some(code)) => format!("[exit {code} · {seconds:.1} s]"),
        Ending::Exited(None) => format!("[stopped by a signal · {seconds:.1} s]"),
        // Only when it could not move to the background: as many already run there.
        Ending::TimedOut(limit) => format!(
            "[stopped after {limit} s, since it could not move to the background; retry with a \
             larger timeout_secs, or stop a command in the background first]"
        ),
        Ending::Cancelled => {
            format!("[stopped after {seconds:.1} s: the person cancelled the turn]")
        }
    }
}

enum Ending {
    Exited(Option<i32>),
    TimedOut(u64),
    /// The turn was cancelled while the command ran.
    Cancelled,
}

#[cfg(test)]
mod tests {

    use glave_ports::block_on;
    use glave_ports::testing::FAKE_SECRET;

    use super::*;

    #[test]
    fn commands_never_see_secrets() {
        let dir = glave_ports::testing::TempDir::new("bash-env").unwrap();
        let mut environment: Vec<_> = std::env::vars_os()
            .filter(|(name, _)| name == "PATH")
            .collect();
        environment.push(("GLAVE_API_KEY".into(), FAKE_SECRET.into()));
        environment.push(("PLAIN_SETTING".into(), "visible".into()));
        let outcome = block_on!(async {
            let workspace = Workspace::open(&dir, environment).await.unwrap();
            let args = BashArgs {
                command: "env".into(),
                timeout_secs: None,
                background: false,
                outside_sandbox: false,
                network: false,
                reason: None,
            };
            let commands = Commands::default();
            bash(&workspace, &commands, args, &crate::cancel::Cancel::new()).await
        });
        assert!(
            outcome.text.contains("PLAIN_SETTING=visible"),
            "{}",
            outcome.text
        );
        assert!(!outcome.text.contains(FAKE_SECRET), "{}", outcome.text);
        assert!(!outcome.text.contains("GLAVE_API_KEY"), "{}", outcome.text);
    }

    #[test]
    fn a_request_to_run_outside_the_sandbox_is_read_as_the_tool_reads_it() {
        let asked = |value: serde_json::Value| {
            outside_sandbox(&Arguments::Json(serde_json::from_value(value).unwrap()))
        };
        let because = |reason: &str| {
            Some(OutsideSandbox {
                reason: Some(reason.to_owned()),
                lifted: Lifted::Sandbox,
            })
        };
        assert_eq!(
            asked(serde_json::json!({ "command": "cargo add serde" })),
            None
        );
        assert_eq!(
            asked(serde_json::json!({ "command": "x", "outside_sandbox": false, "reason": "r" })),
            None
        );
        assert_eq!(
            asked(
                serde_json::json!({ "command": "x", "outside_sandbox": true, "reason": " it writes ~/.cargo\n" })
            ),
            because("it writes ~/.cargo")
        );
        assert_eq!(
            asked(serde_json::json!({ "command": "x", "outside_sandbox": true, "reason": "  " })),
            Some(OutsideSandbox::default())
        );
        let long = asked(serde_json::json!({
            "command": "x", "outside_sandbox": true, "reason": "y".repeat(2_000)
        }));
        assert_eq!(long, because(&"y".repeat(MOST_REASON_CHARS)));
        // Arguments the tool would refuse ask nothing: it never runs them.
        assert_eq!(
            asked(serde_json::json!({ "command": "x", "outside_sandbox": "yes" })),
            None
        );
        let network = asked(serde_json::json!({
            "command": "npm install", "network": true, "reason": "it downloads packages"
        }));
        assert_eq!(
            network,
            Some(OutsideSandbox {
                reason: Some("it downloads packages".to_owned()),
                lifted: Lifted::Network,
            })
        );
        let both = asked(serde_json::json!({
            "command": "x", "network": true, "outside_sandbox": true
        }));
        assert_eq!(
            both.map(|outside| outside.lifted),
            Some(Lifted::Sandbox),
            "outside the sandbox has the network too"
        );
    }
}
