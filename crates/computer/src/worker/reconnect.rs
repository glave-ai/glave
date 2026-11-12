//! A computer's connection, opened again when it dies: through a new bridge into it, with a
//! fresh token given to the computer (the first one only opens connections for its first ten
//! minutes), tried again at each call, since a bridge costs one exec and the computer goes on
//! without one. The call that saw it die is never sent again; the next call goes over the new
//! connection. It is opened again only to the same Pod: one woken again since, by this server or
//! another, is another computer, whose glave knows nothing this connection opened.

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use glave_ports::computer::ComputerError;
use glave_ports::{Clock as _, SystemClock};
use tokio::time::Instant;

use super::TOKEN_LIFETIME;
use super::kubernetes::PersonComputer;
use crate::remote::RemoteComputers;
use crate::token::Token;

/// How long a bridge is tried again while the cluster's API server does not answer, as while
/// it restarts: seconds, at most a minute.
const BRIDGE_AGAIN: Duration = Duration::from_mins(1);
/// How often it is tried in that time.
const BRIDGE_RETRY: Duration = Duration::from_secs(1);

/// One computer's connection, to the Pod it was opened to.
pub(crate) struct Connection {
    remote: Mutex<Arc<RemoteComputers>>,
    /// The uid of the Pod it was opened to, when it could be read.
    pub(crate) pod: Option<String>,
    /// Held while the connection is opened again, so one call opens it and the others use it.
    reopening: tokio::sync::Mutex<()>,
}

/// Connects to glave in `computer` through a new bridge into it, signed in with `token`.
pub(crate) async fn bridged(
    computer: &PersonComputer,
    token: &str,
) -> Result<RemoteComputers, ComputerError> {
    let (read, write) = computer.bridge().await?;
    let remote = RemoteComputers::connect(read, write);
    remote.authenticate(token).await?;
    remote.check_protocol().await?;
    Ok(remote)
}

impl Connection {
    pub(crate) fn new(remote: RemoteComputers, pod: Option<String>) -> Self {
        Self {
            remote: Mutex::new(Arc::new(remote)),
            pod,
            reopening: tokio::sync::Mutex::new(()),
        }
    }

    fn now(&self) -> Result<Arc<RemoteComputers>, ComputerError> {
        self.remote
            .lock()
            .map(|remote| Arc::clone(&remote))
            .map_err(|_| ComputerError::Failed("the computer's connection lock is poisoned".into()))
    }

    /// The connection to use for the next call on `computer`: the open one, or, when it has
    /// died, a new one.
    pub(crate) async fn get(
        &self,
        computer: &PersonComputer,
    ) -> Result<Arc<RemoteComputers>, ComputerError> {
        let remote = self.now()?;
        if remote.closed().is_none() {
            return Ok(remote);
        }
        let _reopening = self.reopening.lock().await;
        // Another call may have opened it again while this one waited.
        let remote = self.now()?;
        let Some(died) = remote.closed() else {
            return Ok(remote);
        };
        if let (Ok(Some(now)), Some(was)) = (computer.running_as().await, &self.pod)
            && now != *was
        {
            return Err(ComputerError::Unreachable(format!(
                "the connection to the computer died ({died}) and it has started again since, \
                 so what ran there before is gone"
            )));
        }
        match bridged_again(computer).await {
            Ok(fresh) => {
                let fresh = Arc::new(fresh);
                if let Ok(mut remote) = self.remote.lock() {
                    *remote = Arc::clone(&fresh);
                }
                Ok(fresh)
            }
            Err(error) => {
                // The cluster's own words, which name its objects, are for the server's log.
                tracing::warn!(why = %error, "a computer's connection could not be opened again");
                Err(ComputerError::Unreachable(format!(
                    "the connection to the computer died ({died}) and could not be opened \
                     again: it did not answer"
                )))
            }
        }
    }
}

/// A new bridge into `computer` with a fresh token, tried again for up to a minute while the
/// API server does not answer, or while another server's fresh token replaced this one before
/// it was presented; a computer that is not there, or not running, fails at once.
pub(crate) async fn bridged_again(
    computer: &PersonComputer,
) -> Result<RemoteComputers, ComputerError> {
    let until = Instant::now() + BRIDGE_AGAIN;
    loop {
        match fresh(computer).await {
            Err(ComputerError::Unreachable(_)) if Instant::now() < until => {
                tokio::time::sleep(BRIDGE_RETRY).await;
            }
            tried => return tried,
        }
    }
}

/// A new bridge into `computer` with a fresh token, tried once.
pub(crate) async fn fresh(computer: &PersonComputer) -> Result<RemoteComputers, ComputerError> {
    let token = renewed(computer).await?;
    bridged(computer, token.secret()).await
}

/// A fresh token, given to `computer`.
async fn renewed(computer: &PersonComputer) -> Result<Token, ComputerError> {
    let token = Token::mint(SystemTime::from(SystemClock.now()), TOKEN_LIFETIME)?;
    computer.renew(&token).await?;
    Ok(token)
}
