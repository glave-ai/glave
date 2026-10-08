//! `bash`: run one command in a fresh shell at the workspace root.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::io::AsyncReadExt as _;
use tokio::process::{Child, ChildStdout, Command};

use super::ToolOutcome;
use super::workspace::Workspace;

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_TIMEOUT_SECS: u64 = 600;
/// Output kept from the start and the end of a long run; the middle is dropped.
const HEAD_BYTES: usize = 8 * 1024;
const TAIL_BYTES: usize = 24 * 1024;
/// How long to keep reading after the command exits, in case a background child it started
/// still holds the output open.
const DRAIN: Duration = Duration::from_millis(500);
/// How long a timed-out command gets between SIGTERM and SIGKILL.
const GRACE: Duration = Duration::from_secs(3);

#[derive(Debug, Deserialize)]
pub(crate) struct BashArgs {
    command: String,
    #[serde(default)]
    timeout_secs: Option<u64>,
}

pub(crate) async fn bash(workspace: &Workspace, args: BashArgs) -> ToolOutcome {
    if args.command.trim().is_empty() {
        return ToolOutcome::error("The command is empty.");
    }
    let limit = args
        .timeout_secs
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .clamp(1, MAX_TIMEOUT_SECS);
    let mut command = Command::new("bash");
    command
        .arg("-c")
        .arg(format!("exec 2>&1\n{}", args.command))
        .current_dir(workspace.root())
        .env_clear()
        .envs(
            workspace
                .environment()
                .iter()
                .map(|(name, value)| (name, value)),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .env("PAGER", "cat")
        .env("GIT_PAGER", "cat")
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let started = Instant::now();
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => return ToolOutcome::error(format!("Could not start bash: {e}")),
    };
    let captured = Arc::new(Mutex::new(Captured::default()));
    let mut reader = child
        .stdout
        .take()
        .map(|stdout| tokio::spawn(collect(stdout, Arc::clone(&captured))));
    let waited = tokio::time::timeout(Duration::from_secs(limit), child.wait()).await;
    let ending = match waited {
        Ok(Ok(status)) => Ending::Exited(status.code()),
        Ok(Err(e)) => return ToolOutcome::error(format!("Could not wait for the command: {e}")),
        Err(_) => {
            stop(&mut child).await;
            Ending::TimedOut(limit)
        }
    };
    if let Some(handle) = reader.as_mut()
        && tokio::time::timeout(DRAIN, &mut *handle).await.is_err()
    {
        handle.abort();
    }
    let output = captured.lock().map(|c| c.render()).unwrap_or_default();
    let seconds = started.elapsed().as_secs_f64();
    let footer = match ending {
        Ending::Exited(Some(code)) => format!("[exit {code} · {seconds:.1} s]"),
        Ending::Exited(None) => format!("[stopped by a signal · {seconds:.1} s]"),
        Ending::TimedOut(limit) => format!(
            "[killed after {limit} s; retry with a larger timeout_secs, or start long-running \
             servers with: nohup <command> > log 2>&1 &]"
        ),
    };
    let body = if output.is_empty() {
        "(no output)".to_owned()
    } else {
        output
    };
    let failed = !matches!(ending, Ending::Exited(Some(0)));
    let text = format!("{}\n{footer}", body.trim_end_matches('\n'));
    if failed {
        ToolOutcome::error(text)
    } else {
        ToolOutcome::ok(text)
    }
}

enum Ending {
    Exited(Option<i32>),
    TimedOut(u64),
}

/// Stops a timed-out command and everything it started: SIGTERM to its process group, then
/// SIGKILL if it is still running after a grace period.
async fn stop(child: &mut Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        signal_group("-TERM", pid).await;
        if tokio::time::timeout(GRACE, child.wait()).await.is_err() {
            signal_group("-KILL", pid).await;
        }
    }
    let _ = child.kill().await;
}

#[cfg(unix)]
async fn signal_group(signal: &str, pid: u32) {
    let _ = Command::new("kill")
        .arg(signal)
        .arg("--")
        .arg(format!("-{pid}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
}

async fn collect(mut stdout: ChildStdout, captured: Arc<Mutex<Captured>>) {
    let mut buffer = vec![0u8; 16 * 1024];
    while let Ok(read) = stdout.read(&mut buffer).await {
        let Some(chunk) = buffer.get(..read).filter(|chunk| !chunk.is_empty()) else {
            break;
        };
        if let Ok(mut captured) = captured.lock() {
            captured.push(chunk);
        }
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
    use super::*;

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn commands_never_see_secrets() {
        let dir = std::env::temp_dir().join(format!("glave-bash-env-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut environment: Vec<_> = std::env::vars_os()
            .filter(|(name, _)| name == "PATH")
            .collect();
        environment.push(("GLAVE_API_KEY".into(), "sk-not-a-real-key".into()));
        environment.push(("PLAIN_SETTING".into(), "visible".into()));
        let outcome = block_on(async {
            let workspace = Workspace::open(&dir, environment).await.unwrap();
            let args = BashArgs {
                command: "env".into(),
                timeout_secs: None,
            };
            bash(&workspace, args).await
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
