//! A worker's connection, opened again when it dies: once, with a fresh token the backend
//! gives the worker (the first one only opens connections for its first ten minutes). The
//! call that saw it die is never sent again; the next call goes over the new connection, so
//! releasing a worker still reaches it to save its work. A worker reached over its standard
//! input and output (no network) cannot be reached again: that stream was its only door.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use glave_ports::computer::ComputerError;

use super::TOKEN_LIFETIME;
use super::backend::Backend;
use crate::remote::RemoteComputers;
use crate::tls::{Fingerprint, connect_tls};
use crate::token::Token;

/// One worker's connection.
pub(crate) struct Connection {
    remote: Mutex<Arc<RemoteComputers>>,
    /// Where the worker listens and the certificate it shows; `None` for a stream.
    address: Option<(SocketAddr, Fingerprint)>,
    /// Why opening the connection again failed: then it is not tried again.
    failed: tokio::sync::Mutex<Option<String>>,
}

impl Connection {
    pub(crate) fn new(remote: RemoteComputers, address: Option<(SocketAddr, Fingerprint)>) -> Self {
        Self {
            remote: Mutex::new(Arc::new(remote)),
            address,
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
        let Some((address, fingerprint)) = &self.address else {
            return Err(ComputerError::Unsupported(
                "the worker has no network: glave reached it over its standard input and \
                 output, which cannot be opened again"
                    .into(),
            ));
        };
        let token = Token::mint(SystemTime::now(), TOKEN_LIFETIME)?;
        backend.renew(name, &token).await?;
        connect_tls(*address, fingerprint, token.secret()).await
    }
}
