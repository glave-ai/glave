//! The task host: the long-running glave for one folder that runs its tasks and schedules, so
//! they go on after the command that started them exits. It is `glave serve --socket`, serving
//! the session API on a Unix socket in GLAVE_HOME that only the person can open. Commands that
//! need it (`--background`, `glave tasks`, `glave schedule`) start it when it is not running,
//! and it exits on its own once nothing runs, no schedule waits and no one is connected.

mod host;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context as _, bail};
use glave_session::protocol::{ApiError, Reply, Request};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

use crate::args::Args;
use crate::home;

pub use host::host;

/// How long a new host has to start listening.
const START_WAIT: Duration = Duration::from_secs(20);
/// How often a starting host is tried.
const START_POLL: Duration = Duration::from_millis(50);

/// The longest socket path every system takes (macOS allows 104 bytes with the terminator).
const MAX_SOCKET: usize = 100;

/// Where the host for `folder` keeps its socket name, log and marks: `GLAVE_HOME/run/<hash of
/// the folder>`, with an extension.
fn run_file(folder: &Path, extension: &str) -> anyhow::Result<PathBuf> {
    Ok(home::folder()?.join("run").join(format!(
        "{:016x}.{extension}",
        fnv1a(folder.as_os_str().as_encoded_bytes())
    )))
}

/// Where the host for `folder` listens: `GLAVE_HOME/run/<hash of the folder>.sock`, or, when
/// that path is too long for a socket, `/tmp/glave-<uid>/<hash of that path>.sock` in a folder
/// only the person may open.
pub fn socket(folder: &Path) -> anyhow::Result<PathBuf> {
    let usual = run_file(folder, "sock")?;
    if usual.as_os_str().len() <= MAX_SOCKET {
        return Ok(usual);
    }
    let uid = {
        use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _};
        let home = home::folder()?;
        // Made as `home::database` makes it, so its owner says who the person is.
        let _ = std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&home);
        std::fs::metadata(&home)
            .context("could not read glave's folder")?
            .uid()
    };
    Ok(PathBuf::from(format!("/tmp/glave-{uid}")).join(format!(
        "{:016x}.sock",
        fnv1a(usual.as_os_str().as_encoded_bytes())
    )))
}

/// Makes the folder `socket` goes in, readable only by the person, and refuses one that
/// someone else could open.
pub async fn private_folder_for(socket: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    let folder = socket.parent().context("the socket has no folder")?;
    let mut builder = tokio::fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder
        .create(folder)
        .await
        .with_context(|| format!("could not create {}", folder.display()))?;
    let made = tokio::fs::metadata(folder).await?;
    let mine = tokio::fs::metadata(home::folder()?).await?.uid();
    if made.uid() != mine || made.permissions().mode() & 0o077 != 0 {
        bail!(
            "{} is not private to you, so the task host will not listen there",
            folder.display()
        );
    }
    Ok(())
}

/// FNV-1a: a hash that stays the same in every glave, so every version finds the same host.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// A connection to the task host.
pub struct Host {
    lines: Lines<BufReader<OwnedReadHalf>>,
    writer: OwnedWriteHalf,
    next_id: i64,
}

impl Host {
    /// The host for `folder`, if one is running.
    pub async fn connect(folder: &Path) -> anyhow::Result<Option<Self>> {
        let path = socket(folder)?;
        Ok(UnixStream::connect(&path).await.ok().map(|stream| {
            let (read, writer) = stream.into_split();
            Self {
                lines: BufReader::new(read).lines(),
                writer,
                next_id: 1,
            }
        }))
    }

    /// The host for `folder`, started with the settings `args` choose when none is running.
    pub async fn connect_or_start(folder: &Path, args: &Args) -> anyhow::Result<Self> {
        if let Some(host) = Self::connect(folder).await? {
            return Ok(host);
        }
        start(folder, args).await?;
        let waited = tokio::time::Instant::now();
        loop {
            if let Some(host) = Self::connect(folder).await? {
                return Ok(host);
            }
            if waited.elapsed() > START_WAIT {
                bail!(
                    "the task host did not start; what it printed is in {}",
                    log_file(folder)?.display()
                );
            }
            tokio::time::sleep(START_POLL).await;
        }
    }

    /// Sends `request` and answers with the reply, or the error the host gave.
    pub async fn request(&mut self, request: Request) -> anyhow::Result<Reply> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let mut message = serde_json::to_value(&request)?;
        if let Value::Object(fields) = &mut message {
            fields.insert("jsonrpc".into(), json!("2.0"));
            fields.insert("id".into(), json!(id));
        }
        let mut line = message.to_string();
        line.push('\n');
        self.writer.write_all(line.as_bytes()).await?;
        self.writer.flush().await?;
        while let Some(line) = self.lines.next_line().await? {
            let message: Value = serde_json::from_str(&line)?;
            if message.get("id") != Some(&json!(id)) {
                // A notification, for clients that follow turns.
                continue;
            }
            if let Some(error) = message.get("error") {
                let error: ApiError = serde_json::from_value(error.clone())?;
                bail!("{error}");
            }
            let result = message.get("result").cloned().unwrap_or(Value::Null);
            return serde_json::from_value(result)
                .context("the task host answered with something this glave cannot read");
        }
        bail!("the task host closed the connection")
    }
}

/// Where the host for `folder` writes what it prints.
pub fn log_file(folder: &Path) -> anyhow::Result<PathBuf> {
    run_file(folder, "log")
}

/// Where a session here remembers which finished tasks it has told the person about.
pub fn seen_file(folder: &Path) -> anyhow::Result<PathBuf> {
    run_file(folder, "seen")
}

/// Starts the host for `folder` on its own, apart from this process and its terminal.
async fn start(folder: &Path, args: &Args) -> anyhow::Result<()> {
    private_folder_for(&log_file(folder)?).await?;
    private_folder_for(&socket(folder)?).await?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file(folder)?)?;
    let glave = std::env::current_exe().context("could not find this glave to start the host")?;
    let mut command = std::process::Command::new(glave);
    command
        .args(["serve", "--socket"])
        .args(settings(args))
        .current_dir(folder)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    // Its own process group: Ctrl-C in this terminal does not reach it.
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let child = command.spawn().context("could not start the task host")?;
    // The host outlives this process on purpose; the system reaps it when it exits.
    drop(child);
    Ok(())
}

/// The options a new host takes from this command: the settings its tasks run with.
fn settings(args: &Args) -> Vec<String> {
    let mut words = Vec::new();
    if let Some(model) = &args.model {
        words.extend(["--model".to_owned(), model.clone()]);
    }
    if let Some(approval) = args.approval {
        words.extend(["--approval".to_owned(), approval.name().to_owned()]);
    }
    if let Some(sandbox) = args.sandbox {
        words.extend(["--sandbox".to_owned(), sandbox.name().to_owned()]);
    }
    match args.network {
        Some(true) => words.push("--network".to_owned()),
        Some(false) => words.push("--no-network".to_owned()),
        None => {}
    }
    if let Some(on) = args.on {
        words.extend(["--on".to_owned(), on.name().to_owned()]);
    }
    if let Some(steps) = args.max_steps {
        words.extend(["--max-steps".to_owned(), steps.to_string()]);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_folder_has_its_own_socket_and_the_name_never_changes() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_ne!(fnv1a(b"/code/one"), fnv1a(b"/code/two"));
    }
}
