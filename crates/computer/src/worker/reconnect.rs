//! A worker's connection, opened again when it dies: through a new bridge into it, with a fresh
//! token the backend gives the worker (the first one only opens connections for its first ten
//! minutes), tried again at each call, since a bridge costs one exec and the worker goes on
//! without one. The call that saw it die is never sent again; the next call goes over the new
//! connection, so releasing a worker still reaches it to save its work.

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use glave_ports::computer::ComputerError;
use glave_ports::{Clock as _, SystemClock};
use tokio::time::Instant;

use super::TOKEN_LIFETIME;
use super::backend::Backend;
use crate::remote::RemoteComputers;
use crate::token::Token;

/// How long a bridge is tried again while the cluster's API server does not answer, as while
/// it restarts: seconds, at most a minute.
const BRIDGE_AGAIN: Duration = Duration::from_mins(1);
/// How often it is tried in that time.
const BRIDGE_RETRY: Duration = Duration::from_secs(1);

/// One worker's connection.
pub(crate) struct Connection {
    remote: Mutex<Arc<RemoteComputers>>,
    /// Held while the connection is opened again, so one call opens it and the others use it.
    reopening: tokio::sync::Mutex<()>,
}

/// Connects to glave in the worker `name` through a new bridge into it, signed in with `token`.
pub(crate) async fn bridged(
    backend: &dyn Backend,
    name: &str,
    token: &str,
) -> Result<RemoteComputers, ComputerError> {
    let (read, write) = backend.bridge(name).await?;
    let remote = RemoteComputers::connect(read, write);
    remote.authenticate(token).await?;
    remote.check_protocol().await?;
    Ok(remote)
}

impl Connection {
    pub(crate) fn new(remote: RemoteComputers) -> Self {
        Self {
            remote: Mutex::new(Arc::new(remote)),
            reopening: tokio::sync::Mutex::new(()),
        }
    }

    fn now(&self) -> Result<Arc<RemoteComputers>, ComputerError> {
        self.remote
            .lock()
            .map(|remote| Arc::clone(&remote))
            .map_err(|_| ComputerError::Failed("the worker's connection lock is poisoned".into()))
    }

    /// The connection to use for the next call on the worker `name`: the open one, or, when
    /// it has died, a new one.
    pub(crate) async fn get(
        &self,
        name: &str,
        backend: &dyn Backend,
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
        match bridged_again(name, backend).await {
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

/// A new bridge into the worker `name` with a fresh token, tried again for up to a minute while
/// the API server does not answer; a worker that is not there, or not running, fails at once.
async fn bridged_again(
    name: &str,
    backend: &dyn Backend,
) -> Result<RemoteComputers, ComputerError> {
    let until = Instant::now() + BRIDGE_AGAIN;
    loop {
        let tried = match renewed(name, backend).await {
            Ok(token) => bridged(backend, name, token.secret()).await,
            Err(error) => Err(error),
        };
        match tried {
            Err(ComputerError::Unreachable(_)) if Instant::now() < until => {
                tokio::time::sleep(BRIDGE_RETRY).await;
            }
            tried => return tried,
        }
    }
}

/// A fresh token, given to the worker `name`.
async fn renewed(name: &str, backend: &dyn Backend) -> Result<Token, ComputerError> {
    let token = Token::mint(SystemTime::from(SystemClock.now()), TOKEN_LIFETIME)?;
    backend.renew(name, &token).await?;
    Ok(token)
}
