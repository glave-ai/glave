//! Signing browsers in on the person's own computer (`specs/one-server.md`, "Finding it"): a
//! program here asks for a link's one-time code, the browser trades it for a session of its
//! own, and ends it, or every other browser's, when the person signs out. Sessions are only
//! ever the local person's: nobody an identity provider signs in gets one, so a browser's
//! sign-in through a provider ends when the provider stops renewing it.

use glave_event::AuthMethod;
use glave_identity::signin::{BrowserError, Browsers};
use glave_ports::{BlobStore, EventLog, ModelProvider};

use crate::Caller;
use crate::protocol::{ApiError, Reply, code};
use crate::providers::local;
use crate::server::Server;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The server's id, where it signs browsers in.
    pub(crate) fn server_id(&self) -> Option<String> {
        self.inner
            .browsers
            .as_ref()
            .map(|browsers| browsers.server())
    }

    /// Answers `signin/link`: a code for the person on this computer, asked for by a program
    /// signed in as them, never by a browser with a session of its own.
    pub(crate) fn link(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let browsers = self.browsers()?;
        let by_program = caller
            .auth
            .as_ref()
            .is_none_or(|auth| auth.method == AuthMethod::Local && auth.session.is_none());
        if !by_program {
            return Err(ApiError::new(
                code::NOT_ALLOWED,
                "only glave on this computer makes sign-in links",
            ));
        }
        let code = browsers.link().map_err(|e| failed(&e))?;
        Ok(Reply::Link { code })
    }

    /// Answers `signin/redeem`: `code`, once, for a session with this server.
    pub(crate) async fn redeem(&self, code: &str) -> Result<Reply, ApiError> {
        let browsers = self.browsers()?;
        let session = browsers.redeem(code).await.map_err(|e| failed(&e))?;
        Ok(Reply::Browser {
            session: session.token,
            server: browsers.server(),
        })
    }

    /// Answers `signin/end`: ends the caller's own browser session.
    pub(crate) async fn end_session(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let browsers = self.browsers()?;
        let session = session_of(caller).ok_or_else(|| {
            ApiError::new(
                code::INVALID_PARAMS,
                "this is not a browser's sign-in, so there is none to end",
            )
        })?;
        browsers.end(session).await.map_err(|e| failed(&e))?;
        Ok(Reply::Done {})
    }

    /// Answers `signin/end_others`: ends every browser session but the caller's own, for the
    /// person on this computer.
    pub(crate) async fn end_others(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let browsers = self.browsers()?;
        if !local(caller) {
            return Err(ApiError::new(
                code::NOT_ALLOWED,
                "only the person on this computer signs its browsers out",
            ));
        }
        browsers
            .end_others(session_of(caller))
            .await
            .map_err(|e| failed(&e))?;
        Ok(Reply::Done {})
    }

    fn browsers(&self) -> Result<&Browsers, ApiError> {
        self.inner.browsers.as_deref().ok_or_else(|| {
            ApiError::new(
                code::NOT_AVAILABLE,
                "this server signs no browser in: open glave on its computer",
            )
        })
    }
}

/// The browser session `caller` signed in with, if they did.
fn session_of(caller: &Caller) -> Option<&str> {
    caller
        .auth
        .as_ref()
        .filter(|auth| auth.method == AuthMethod::Local)
        .and_then(|auth| auth.session.as_deref())
}

fn failed(error: &BrowserError) -> ApiError {
    match error {
        BrowserError::Code | BrowserError::Ended => {
            ApiError::new(code::NO_SIGN_IN, error.to_string())
        }
        // Where and why go to the server's log; the person is told what to do.
        BrowserError::Keep { .. } | BrowserError::Random(_) => {
            tracing::warn!(%error, "could not sign a browser in");
            ApiError::new(
                code::INTERNAL,
                "glave could not keep this browser's sign-in: try again",
            )
        }
    }
}
