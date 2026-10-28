//! A watch's caller's access, for as long as it follows something: a thread's watch, a turn's
//! stream, a list's watch. Each is let through once, as it begins; what can end the caller's
//! access while it runs is their browser's sign-in, ended here (signing out, signing other
//! browsers out, removing the passkey that signed it in, or a new sign-in past the most
//! kept). The hub tells every watch when one may have ended, and each that follows a browser's
//! session checks its own, so the watch ends at once rather than when its connection closes.
//! Projects have one owner, who keeps theirs: removing a project archives its threads, still
//! theirs to read.

use std::future::pending;
use std::sync::Arc;

use glave_identity::signin::{BrowserError, Browsers};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};
use tokio::sync::watch;

use crate::browsers::session_of;
use crate::protocol::{ApiError, code};
use crate::server::Server;

/// The browser session a watch's caller signed in with, and when to look at it again.
pub(crate) struct Access {
    browsers: Arc<Browsers>,
    session: String,
    sign_ins: watch::Receiver<()>,
    /// Whether a sign-in may have ended since the session was last looked at: waiting for
    /// the change takes it as seen.
    heard: bool,
}

impl Access {
    /// Waits until a sign-in here may have ended, for a watch whose caller's `access` something
    /// here can end; never for any other, nor once the server has gone.
    pub(crate) async fn changed(access: &mut Option<Self>) {
        let heard = match access {
            Some(access) => {
                access.heard = access.sign_ins.changed().await.is_ok();
                access.heard
            }
            None => false,
        };
        if !heard {
            pending::<()>().await;
        }
    }

    /// Why the caller may no longer follow what they watch, once a sign-in here may have ended
    /// and theirs has.
    pub(crate) fn ended(&mut self) -> Option<ApiError> {
        let heard = std::mem::take(&mut self.heard) || self.sign_ins.has_changed().unwrap_or(false);
        if !heard {
            return None;
        }
        self.sign_ins.mark_unchanged();
        self.refused()
    }

    fn refused(&self) -> Option<ApiError> {
        (!self.browsers.signed_in(&self.session)).then(|| ApiError::from(BrowserError::Ended))
    }
}

/// Whether `error` ended a watch because its caller's access ended.
pub(crate) fn access_ended(error: &ApiError) -> bool {
    error.code == code::NO_SIGN_IN
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The access a watch for `caller` follows: their browser's session, when they signed in
    /// with one here; `None` for anyone else, whose sign-in nothing here ends. Refused when it
    /// has ended already, checked after hearing begins so no ending falls between.
    pub(crate) fn access(&self, caller: &Caller) -> Result<Option<Access>, ApiError> {
        let (Some(browsers), Some(session)) = (self.inner.browsers.as_ref(), session_of(caller))
        else {
            return Ok(None);
        };
        let access = Access {
            browsers: Arc::clone(browsers),
            session: session.to_owned(),
            sign_ins: self.inner.hub.sign_ins(),
            heard: false,
        };
        match access.refused() {
            Some(refused) => Err(refused),
            None => Ok(Some(access)),
        }
    }
}
