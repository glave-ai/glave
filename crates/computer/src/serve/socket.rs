//! A computer served on a Unix socket, and the bridge that carries one connection to it: how a
//! computer in a Pod is reached (`specs/computers.md`, decision 12). Each connection from a
//! harness is one exec of `glave computer bridge <socket>` through the cluster's API server,
//! whose standard input and output carry the protocol to the socket, so nothing listens on the
//! Pod's network, and an exec that drops ends only its bridge, never the computer.

use std::convert::Infallible;
use std::os::unix::fs::{FileTypeExt as _, PermissionsExt as _};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use glave_ports::computer::Computers;
use glave_ports::{Clock as _, SystemClock};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::task::JoinSet;
use tokio::time::Instant;

use super::idle::Activity;
use super::{Idle, MAX_CONNECTIONS, serving};
use crate::token::Tokens;

/// How long a bridge waits for the computer's socket to be there and take it: the glave just
/// copied into its Pod serves within a second of arriving.
const SOCKET_WAIT: Duration = Duration::from_secs(30);
/// How often a bridge tries the socket while it waits.
const SOCKET_RETRY: Duration = Duration::from_millis(100);

/// Listens on a Unix socket at `path`, open to this user alone. A socket left there before
/// (by a glave that ended) is replaced; anything else at the path is refused.
pub fn listen_socket(path: &Path) -> std::io::Result<UnixListener> {
    match std::fs::symlink_metadata(path) {
        Ok(found) if found.file_type().is_socket() => std::fs::remove_file(path)?,
        Ok(_) => {
            return Err(std::io::Error::other(format!(
                "{} is there and is not a socket",
                path.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Serves `computers` on a Unix socket, to clients that first present the token `tokens` holds,
/// before it expires: how a computer in a Pod is reached, each connection a bridge. Runs until
/// accepting fails, or, with `idle`, until no request has run for that long and nothing is
/// busy.
pub async fn serve_socket<C: Computers + 'static>(
    computers: Arc<C>,
    listener: UnixListener,
    tokens: impl Into<Tokens>,
    idle: Option<Idle>,
) -> std::io::Result<()> {
    let tokens = tokens.into();
    let activity = idle.as_ref().map(|_| Activity::new());
    let mut connections = JoinSet::new();
    loop {
        while connections.len() >= MAX_CONNECTIONS {
            connections.join_next().await;
        }
        let stream = match (&idle, &activity) {
            (Some(idle), Some(activity)) => tokio::select! {
                accepted = listener.accept() => accepted?.0,
                () = idle.reached(activity) => return Ok(()),
            },
            _ => listener.accept().await?.0,
        };
        let (computers, tokens) = (Arc::clone(&computers), tokens.clone());
        let activity = activity.clone();
        connections.spawn(async move {
            // The token as it is now: the side that minted it may have renewed it.
            let token = tokens.current(SystemTime::from(SystemClock.now())).await?;
            let (read, write) = stream.into_split();
            serving(
                computers,
                BufReader::new(read),
                write,
                Some(token),
                activity,
            )
            .await
        });
        while connections.try_join_next().is_some() {}
    }
}

/// Carries one connection between `input` and `output` (a bridge's standard input and output)
/// and the computer serving on the Unix socket at `path`, waiting up to 30 seconds for the
/// socket to be there. It ends when the computer closes the connection, which it does once
/// `input` has ended, or when `output` can no longer be written.
pub async fn bridge<R, W>(path: &Path, input: R, output: W) -> std::io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let (mut from, mut to) = reached(path).await?.into_split();
    let (mut input, mut output) = (input, output);
    let up = async move {
        let _ = tokio::io::copy(&mut input, &mut to).await;
        // The computer sees its input end, stops what the connection runs and closes it.
        let _ = to.shutdown().await;
        std::future::pending::<Infallible>().await
    };
    let down = async move {
        let copied = tokio::io::copy(&mut from, &mut output).await;
        output.flush().await?;
        copied.map(drop)
    };
    tokio::select! {
        copied = down => copied,
        never = up => match never {},
    }
}

/// The socket at `path`, once it is there and takes the connection, within [`SOCKET_WAIT`].
async fn reached(path: &Path) -> std::io::Result<UnixStream> {
    let until = Instant::now() + SOCKET_WAIT;
    loop {
        let error = match UnixStream::connect(path).await {
            Ok(stream) => return Ok(stream),
            Err(error) => error,
        };
        let starting = matches!(
            error.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
        );
        if starting && Instant::now() < until {
            tokio::time::sleep(SOCKET_RETRY).await;
            continue;
        }
        let waited = if starting {
            format!(" within {}", glave_ports::spoken(SOCKET_WAIT))
        } else {
            String::new()
        };
        return Err(std::io::Error::new(
            error.kind(),
            format!(
                "no computer took a connection at {}{waited}: {error}",
                path.display()
            ),
        ));
    }
}
