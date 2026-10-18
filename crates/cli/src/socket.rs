//! The Unix socket glave serves the session API on for the person alone: the person's server's
//! (`GLAVE_HOME/run/server.sock`), `0600` in a folder only the person may open; where its path
//! is too long for a socket, it is `/tmp/glave-<uid>/<hash of that path>.sock` instead. A
//! client checks the folder's owner and mode and the listener's user before it sends anything,
//! since requests may carry a token.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use glave_session::protocol::{Reply, Request};
use glave_session::rpc;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{UnixListener, UnixStream};

use crate::home;

/// The longest socket path every system takes (macOS allows 104 bytes with the terminator).
const MAX_SOCKET: usize = 100;

/// Where a socket meant to be at `usual` listens: there, or in `/tmp/glave-<uid>` when that
/// path is too long for a socket.
pub fn place(usual: PathBuf) -> anyhow::Result<PathBuf> {
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

/// The person, as glave's folder's owner says.
async fn person() -> anyhow::Result<u32> {
    use std::os::unix::fs::MetadataExt as _;
    Ok(tokio::fs::metadata(home::folder()?).await?.uid())
}

/// Makes the folder `path` goes in, readable only by the person, and refuses one that someone
/// else could open.
pub async fn private_folder_for(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    let folder = path.parent().context("the socket has no folder")?;
    let mut builder = tokio::fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder
        .create(folder)
        .await
        .with_context(|| format!("could not create {}", folder.display()))?;
    let made = tokio::fs::metadata(folder).await?;
    if !private(made.uid(), made.permissions().mode(), person().await?) {
        bail!(
            "{} is not private to you, so glave will not listen there",
            folder.display()
        );
    }
    Ok(())
}

/// Listens at `path`, `0600` in a folder private to the person, in place of a socket file a
/// server that stopped without removing it left there: only for the lock's holder.
pub async fn listen(path: &Path) -> anyhow::Result<UnixListener> {
    use std::os::unix::fs::PermissionsExt as _;
    private_folder_for(path).await?;
    let _ = tokio::fs::remove_file(path).await;
    let listener = UnixListener::bind(path)
        .with_context(|| format!("could not listen on {}", path.display()))?;
    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await?;
    Ok(listener)
}

/// Whether a folder `owner` owns with `mode` is the person's (`mine`) alone.
fn private(owner: u32, mode: u32, mine: u32) -> bool {
    // Read, write or search by the group or anyone else.
    let others = mode & 0o077;
    owner == mine && others == 0
}

/// Whether a socket in a folder `owner` owns with `mode`, with `listener` listening on it, is
/// the person's (`mine`) own.
fn own(owner: u32, mode: u32, listener: u32, mine: u32) -> bool {
    private(owner, mode, mine) && listener == mine
}

/// FNV-1a: a hash that stays the same in every glave, so every version finds the same socket.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// A connection to a glave serving on one of the person's sockets.
pub struct Connection {
    lines: Lines<BufReader<OwnedReadHalf>>,
    writer: OwnedWriteHalf,
    next_id: u64,
    /// The process listening, as the system says.
    pid: Option<i32>,
}

impl Connection {
    /// The glave serving at `path` (`what` names it in errors), if one answers there. Only the
    /// person's own: a socket in a folder others may open, or one another user listens on, is
    /// refused before anything is sent.
    pub async fn open(path: &Path, what: &str) -> anyhow::Result<Option<Self>> {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        let Ok(stream) = UnixStream::connect(path).await else {
            return Ok(None);
        };
        let mine = person().await?;
        let around = path.parent().context("the socket has no folder")?;
        let around = tokio::fs::metadata(around).await?;
        let peer = stream
            .peer_cred()
            .with_context(|| format!("could not tell who listens at {what}'s socket"))?;
        if !own(around.uid(), around.permissions().mode(), peer.uid(), mine) {
            bail!(
                "{} is not your own {what}'s, so glave did not use it",
                path.display()
            );
        }
        let (read, writer) = stream.into_split();
        Ok(Some(Self {
            lines: BufReader::new(read).lines(),
            writer,
            next_id: 1,
            pid: peer.pid(),
        }))
    }

    /// The process that listens, where the system tells.
    pub fn pid(&self) -> Option<i32> {
        self.pid
    }

    /// Sends `request` and answers with the reply, or the error the server gave.
    pub async fn request(&mut self, request: Request) -> anyhow::Result<Reply> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let mut line = rpc::message(id, &request).to_string();
        line.push('\n');
        self.writer.write_all(line.as_bytes()).await?;
        self.writer.flush().await?;
        while let Some(line) = self.lines.next_line().await? {
            let message: Value = serde_json::from_str(&line)?;
            if message.get("id") != Some(&json!(id)) {
                // A notification, for clients that follow turns.
                continue;
            }
            return rpc::outcome(&message).map_err(anyhow::Error::new);
        }
        bail!("glave closed the connection")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_path_has_its_own_hash_and_the_name_never_changes() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_ne!(fnv1a(b"/code/one"), fnv1a(b"/code/two"));
    }

    #[test]
    fn only_a_folder_the_person_owns_alone_is_private() {
        assert!(private(501, 0o700, 501));
        assert!(private(501, 0o40700, 501), "a folder's type bits aside");
        assert!(!private(501, 0o750, 501), "the group may open it");
        assert!(!private(501, 0o701, 501), "anyone may open it");
        assert!(!private(0, 0o700, 501), "another user's");
    }

    #[test]
    fn a_socket_another_user_listens_on_is_not_the_person_s() {
        assert!(own(501, 0o700, 501, 501));
        assert!(!own(501, 0o700, 502, 501), "another user listens");
        assert!(!own(501, 0o755, 501, 501), "others may open its folder");
    }
}
