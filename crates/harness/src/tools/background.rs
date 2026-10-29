//! `command_output`: what a command in the background printed since the model last looked,
//! whether it still runs, and stopping it; given no command, the commands still running. And
//! what `bash` answers when it starts a command there, or one moves there at its timeout.

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use glave_event::CommandEnding;
use glave_ports::computer::{ComputerError, Ending, Finished, KEPT_OUTPUT, MAX_WAIT, Printed};
use glave_ports::{Cancellation, unless_cancelled};
use serde::Deserialize;

use super::ToolOutcome;
use super::captured::Captured;
use super::workspace::{Workspace, explain};
use crate::background::Commands;

/// How long a command started in the background is watched before the call answers: long
/// enough for most servers to print their address, or fail to start, and short enough not to
/// hold up the turn.
const FIRST_LOOK: Duration = Duration::from_secs(2);

/// What reading again or stopping looks like, for the model.
const NEXT: &str = "see what it prints next with command_output, or stop it with command_output \
     and stop";

/// What the model is told of `command_output`, and the arguments it takes.
pub(super) fn spec() -> (&'static str, serde_json::Value) {
    (
        "See what a command running in the background printed since you last looked, and \
         whether it is still running. wait_secs waits up to that long for it to end, answering \
         as soon as it does. stop stops it and everything it started. Without command, lists \
         the commands running in the background.",
        serde_json::json!({"type": "object", "properties": {
            "command": {"type": "integer", "minimum": 1, "description": "Its number, as bash gave it"},
            "wait_secs": {"type": "integer", "minimum": 0, "maximum": 600},
            "stop": {"type": "boolean", "description": "Stop it"}
        }}),
    )
}

#[derive(Debug, Deserialize)]
pub(crate) struct OutputArgs {
    /// The command's number, as `bash` gave it; none lists them.
    #[serde(default)]
    command: Option<u32>,
    #[serde(default)]
    wait_secs: Option<u64>,
    #[serde(default)]
    stop: bool,
}

/// The number of the command a `command_output` call names, read as the tool reads it.
pub(crate) fn named(arguments: &glave_event::Arguments) -> Option<u32> {
    let args: OutputArgs = super::parse(super::Tool::CommandOutput, arguments, None).ok()?;
    args.command
}

pub(crate) async fn command_output(
    commands: &Commands,
    args: OutputArgs,
    room: usize,
    cancel: &dyn Cancellation,
) -> ToolOutcome {
    let Some(number) = args.command else {
        return ToolOutcome::ok(listing(commands));
    };
    let Some(entry) = commands.numbered(number) else {
        let listed = listing(commands);
        return ToolOutcome::error(format!("There is no command {number}. {listed}"));
    };
    let read = if args.stop {
        commands.stop_entry(&entry, None).await
    } else {
        let wait = Duration::from_secs(args.wait_secs.unwrap_or(0)).min(MAX_WAIT);
        match unless_cancelled(cancel, commands.read(&entry, wait)).await {
            Some(read) => read,
            None => {
                return ToolOutcome::error(format!(
                    "Stopped waiting: the person cancelled the turn. Command {number} is still \
                     running."
                ));
            }
        }
    };
    match read {
        Ok(printed) => shown(number, &printed, room),
        Err(ComputerError::NotFound(_)) => {
            let ended = commands.numbered(number).and_then(|entry| entry.ended);
            ToolOutcome::ok(format!(
                "Command {number} {}: there is no more of its output.",
                ended_as(ended.as_ref())
            ))
        }
        Err(error) => ToolOutcome::error(explain(&error)),
    }
}

/// Starts `script` in the background, and answers its number and what it printed first.
pub(crate) async fn start(
    workspace: &Workspace,
    commands: &Commands,
    script: String,
    room: usize,
    cancel: &dyn Cancellation,
) -> ToolOutcome {
    let id = match workspace.start(script.clone()).await {
        Ok(id) => id,
        Err(ComputerError::Refused(why)) => {
            return ToolOutcome::error(format!(
                "Not started: {why}. command_output with no command lists them, and with a \
                 command and stop stops one."
            ));
        }
        Err(error) => return ToolOutcome::error(explain(&error)),
    };
    let computers = Arc::clone(workspace.computers());
    let number = commands.add(id, workspace.info().id, computers, script, false);
    let first = match commands.numbered(number) {
        Some(entry) => unless_cancelled(cancel, commands.read(&entry, FIRST_LOOK)).await,
        None => None,
    };
    match first {
        Some(Ok(printed)) if printed.ended.is_some() => {
            let shown = shown(number, &printed, room);
            ToolOutcome {
                text: format!(
                    "Command {number} started in the background and ended at once.\n{}",
                    shown.text
                ),
                ..shown
            }
        }
        Some(Ok(printed)) => ToolOutcome::ok(format!(
            "Started command {number} in the background.\n{}\n[command {number} is still \
             running: {NEXT}]",
            body(&printed.text, "(no output yet)")
        )),
        _ => ToolOutcome::ok(format!(
            "Started command {number} in the background.\n[command {number} is still running: \
             {NEXT}]"
        )),
    }
}

/// What `bash` answers when its command reached its timeout and moved to the background.
pub(crate) fn moved(number: u32, after_secs: u64, output: &str) -> ToolOutcome {
    ToolOutcome::ok(format!(
        "{}\n[still running after {after_secs} s, so it now runs in the background as command \
         {number}: {NEXT}; wait_secs waits for it to end]",
        body(output, "(no output yet)")
    ))
}

/// What a read or a stop showed: what the command printed since the last look, and how it
/// is now, in `room` bytes.
fn shown(number: u32, printed: &Printed, room: usize) -> ToolOutcome {
    let mut text = String::new();
    if printed.dropped > 0 {
        let _ = writeln!(
            text,
            "[… {} bytes of earlier output were dropped: only the latest {} MB is kept …]",
            printed.dropped,
            KEPT_OUTPUT / (1024 * 1024)
        );
    }
    let captured = Captured::of(&printed.text, room).render();
    text.push_str(&body(&captured, "(no new output)"));
    text.push('\n');
    let failed = printed.ended.is_some_and(|ended| {
        matches!(ended.ending, Ending::Exited { code } if code != 0)
            || ended.ending == Ending::Signalled
    });
    text.push_str(&state(number, printed.ended));
    if failed {
        ToolOutcome::error(text)
    } else {
        ToolOutcome::ok(text)
    }
}

/// Output as the model reads it, or `none` when there is none.
fn body(output: &str, none: &str) -> String {
    let output = output.trim_end_matches('\n');
    if output.is_empty() {
        none.to_owned()
    } else {
        output.to_owned()
    }
}

/// The line after a command's output: whether it still runs, or how it ended.
fn state(number: u32, ended: Option<Finished>) -> String {
    let Some(Finished { ending, millis, .. }) = ended else {
        return format!("[command {number} is still running]");
    };
    let after = took(millis);
    match ending {
        Ending::Exited { code } => {
            format!("[command {number} exited with code {code} after {after}]")
        }
        Ending::Signalled => format!("[command {number} was stopped by a signal after {after}]"),
        Ending::Cancelled => {
            format!("[command {number} was stopped after {after}, with everything it started]")
        }
        Ending::Idle { .. } => {
            format!("[command {number} was stopped after an hour in which nothing read its output]")
        }
        Ending::TimedOut { after_secs } => {
            format!("[command {number} was stopped at its time limit, after {after_secs} s]")
        }
        Ending::Backgrounded { .. } => format!("[command {number} is still running]"),
    }
}

/// How long something took, for the model: seconds, minutes or hours.
fn took(millis: u64) -> String {
    let seconds = Duration::from_millis(millis).as_secs_f64();
    if seconds < 60.0 {
        format!("{seconds:.1} s")
    } else if seconds < 3600.0 {
        format!("{:.1} min", seconds / 60.0)
    } else {
        format!("{:.1} h", seconds / 3600.0)
    }
}

/// How a command glave knows has ended did, after "Command 2".
fn ended_as(ending: Option<&CommandEnding>) -> String {
    match ending {
        Some(CommandEnding::Exited { code }) => format!("exited with code {code}"),
        Some(CommandEnding::Signalled) => "was stopped by a signal".to_owned(),
        Some(CommandEnding::Stopped { by: None }) => "was stopped".to_owned(),
        Some(CommandEnding::Stopped { by: Some(_) }) => "was stopped by the person".to_owned(),
        Some(CommandEnding::Released) => "was stopped by glave".to_owned(),
        Some(CommandEnding::Idle) => {
            "was stopped after an hour in which nothing read its output".to_owned()
        }
        Some(CommandEnding::TimedOut) => "was stopped at its time limit".to_owned(),
        Some(CommandEnding::Gone) | None => "has ended".to_owned(),
    }
}

/// The commands still running, as the model is told them.
fn listing(commands: &Commands) -> String {
    let running = commands.running();
    if running.is_empty() {
        return "No command is running in the background.".to_owned();
    }
    let mut text = "Commands running in the background:".to_owned();
    for entry in running {
        let _ = write!(text, "\n{}: {}", entry.number, entry.script);
    }
    text
}

#[cfg(test)]
mod tests;
