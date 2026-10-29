//! A worker's connection, opened again when it dies: once, with a fresh token the backend
//! gives the worker (the first one only opens connections for its first ten minutes). The
//! call that saw it die is never sent again; the next call goes over the new connection, so
//! releasing a worker still reaches it to save its work. A worker reached over its standard
//! input and output (no network, on Docker) cannot be reached again: that stream was its only
//! door.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use glave_ports::computer::ComputerError;
use glave_ports::{Clock as _, SystemClock};

use super::TOKEN_LIFETIME;
use super::backend::{Backend, Bridge};
use crate::remote::RemoteComputers;
use crate::tls::{Fingerprint, connect_tls};
use crate::token::Token;

/// How a worker's connection is opened again.
pub(crate) enum Door {
    /// Over TLS at the address it listens on, trusting only the certificate it shows.
    Tls(SocketAddr, Fingerprint),
    /// Through a new bridge into it.
    Bridge(Bridge),
    /// Not at all: its standard input and output were its only door.
    None,
}

/// One worker's connection.
pub(crate) struct Connection {
    remote: Mutex<Arc<RemoteComputers>>,
    door: Door,
    /// Why opening the connection again failed: then it is not tried again.
    failed: tokio::sync::Mutex<Option<String>>,
}

/// Connects to glave in a worker through a new `bridge` into it, signed in with `token`.
pub(crate) async fn bridged(
    bridge: &Bridge,
    token: &str,
) -> Result<RemoteComputers, ComputerError> {
    let (read, write) = bridge().await?;
    let remote = RemoteComputers::connect(read, write);
    remote.authenticate(token).await?;
    remote.check_protocol().await?;
    Ok(remote)
}

impl Connection {
    pub(crate) fn new(remote: RemoteComputers, door: Door) -> Self {
        Self {
            remote: Mutex::new(Arc::new(remote)),
            door,
            failed: tokio::sync::Mutex::new(None),
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
        let mut failed = self.failed.lock().await;
        // Another call may have opened it again while this one waited.
        let remote = self.now()?;
        let Some(died) = remote.closed() else {
            return Ok(remote);
        };
        if let Some(why) = failed.as_ref() {
            return Err(ComputerError::Unreachable(format!(
                "the connection to worker {name} died ({died}) and could not be opened again: \
                 {why}"
            )));
        }
        match self.again(name, backend).await {
            Ok(fresh) => {
                let fresh = Arc::new(fresh);
                if let Ok(mut remote) = self.remote.lock() {
                    *remote = Arc::clone(&fresh);
                }
                Ok(fresh)
            }
            Err(error) => {
                let why = error.to_string();
                *failed = Some(why.clone());
                Err(ComputerError::Unreachable(format!(
                    "the connection to worker {name} died ({died}) and could not be opened \
                     again: {why}"
                )))
            }
        }
    }

    /// Gives the worker a fresh token and connects with it.
    async fn again(
        &self,
        name: &str,
        backend: &dyn Backend,
    ) -> Result<RemoteComputers, ComputerError> {
        match &self.door {
            Door::Tls(address, fingerprint) => {
                let token = renewed(name, backend).await?;
                connect_tls(*address, fingerprint, token.secret()).await
            }
            Door::Bridge(bridge) => bridged(bridge, renewed(name, backend).await?.secret()).await,
            Door::None => Err(ComputerError::Unsupported(
                "the worker has no network: glave reached it over its standard input and \
                 output, which cannot be opened again"
                    .into(),
            )),
        }
    }
}

/// A fresh token, given to the worker `name`.
async fn renewed(name: &str, backend: &dyn Backend) -> Result<Token, ComputerError> {
    let token = Token::mint(SystemTime::from(SystemClock.now()), TOKEN_LIFETIME)?;
    backend.renew(name, &token).await?;
    Ok(token)
}
