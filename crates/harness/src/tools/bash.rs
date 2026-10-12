//! `bash`: run one command in a fresh shell at the workspace root, on the workspace's
//! computer, keeping the start and the end of what it prints.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::Mutex;
use std::time::Duration;

use glave_ports::{BoxFuture, Cancellation, Ending as Exit, Output};
use serde::Deserialize;

use super::ToolOutcome;
use super::workspace::Workspace;
use crate::sandbox::blocked_note;

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_TIMEOUT_SECS: u64 = 600;
/// Output kept from the start and the end of a long run; the middle is dropped.
const HEAD_BYTES: usize = 8 * 1024;
const TAIL_BYTES: usize = 24 * 1024;

#[derive(Debug, Deserialize)]
pub(crate) struct BashArgs {
    command: String,
    #[serde(default)]
    timeout_secs: Option<u64>,
}

pub(crate) async fn bash(
    workspace: &Workspace,
    args: BashArgs,
    cancel: &dyn Cancellation,
) -> ToolOutcome {
    if args.command.trim().is_empty() {
        return ToolOutcome::error("The command is empty.");
    }
    let limit = args
        .timeout_secs
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .clamp(1, MAX_TIMEOUT_SECS);
    let captured = Capture::default();
    let finished = match workspace.run(args.command, limit, &captured, cancel).await {
        Ok(finished) => finished,
        Err(error) => return ToolOutcome::error(sentence(error.message())),
    };
    let ending = match finished.ending {
        Exit::Exited { code } => Ending::Exited(Some(code)),
        Exit::Signalled => Ending::Exited(None),
        Exit::TimedOut { after_secs } => Ending::TimedOut(after_secs),
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
        Ending::TimedOut(limit) => format!(
            "[killed after {limit} s; retry with a larger timeout_secs, or start long-running \
             servers with: nohup <command> > log 2>&1 &]"
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

/// The command's output as it arrives, keeping its start and its end.
#[derive(Default)]
struct Capture(Mutex<Captured>);

impl Capture {
    fn render(&self) -> String {
        self.0.lock().map(|c| c.render()).unwrap_or_default()
    }
}

impl Output for Capture {
    fn write<'a>(&'a self, text: &'a str) -> BoxFuture<'a, ()> {
        if let Ok(mut captured) = self.0.lock() {
            captured.push(text.as_bytes());
        }
        Box::pin(std::future::ready(()))
    }
}

/// The start and the end of a command's output, with a count of what fell in between.
#[derive(Debug, Default)]
struct Captured {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    total: usize,
}

impl Captured {
    fn push(&mut self, bytes: &[u8]) {
        self.total = self.total.saturating_add(bytes.len());
        let room = HEAD_BYTES.saturating_sub(self.head.len());
        let (to_head, to_tail) = bytes.split_at(room.min(bytes.len()));
        self.head.extend_from_slice(to_head);
        self.tail.extend(to_tail);
        while self.tail.len() > TAIL_BYTES {
            self.tail.pop_front();
        }
    }

    fn render(&self) -> String {
        let mut text = String::from_utf8_lossy(&self.head).into_owned();
        let kept = self.head.len().saturating_add(self.tail.len());
        if self.total > kept {
            let _ = write!(
                text,
                "\n[… {} bytes of output omitted …]\n",
                self.total.saturating_sub(kept)
            );
        }
        let tail: Vec<u8> = self.tail.iter().copied().collect();
        text.push_str(&String::from_utf8_lossy(&tail));
        text
    }
}

#[cfg(test)]
mod tests {

    use glave_ports::block_on;

    use super::*;

    #[test]
    fn commands_never_see_secrets() {
        let dir = std::env::temp_dir().join(format!("glave-bash-env-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut environment: Vec<_> = std::env::vars_os()
            .filter(|(name, _)| name == "PATH")
            .collect();
        environment.push(("GLAVE_API_KEY".into(), "sk-not-a-real-key".into()));
        environment.push(("PLAIN_SETTING".into(), "visible".into()));
        let outcome = block_on!(async {
            let workspace = Workspace::open(&dir, environment).await.unwrap();
            let args = BashArgs {
                command: "env".into(),
                timeout_secs: None,
            };
            bash(&workspace, args, &crate::cancel::Cancel::new()).await
        });
        assert!(
            outcome.text.contains("PLAIN_SETTING=visible"),
            "{}",
            outcome.text
        );
        assert!(
            !outcome.text.contains("sk-not-a-real-key"),
            "{}",
            outcome.text
        );
        assert!(!outcome.text.contains("GLAVE_API_KEY"), "{}", outcome.text);
    }

    #[test]
    fn long_output_keeps_head_and_tail() {
        let mut captured = Captured::default();
        captured.push(&vec![b'a'; HEAD_BYTES + 100]);
        captured.push(&vec![b'z'; TAIL_BYTES + 50]);
        let text = captured.render();
        assert!(text.starts_with('a'));
        assert!(text.ends_with('z'));
        assert!(text.contains("[… 150 bytes of output omitted …]"));
    }

    #[test]
    fn short_output_is_complete() {
        let mut captured = Captured::default();
        captured.push(b"hello\n");
        captured.push(b"world\n");
        assert_eq!(captured.render(), "hello\nworld\n");
    }
}
