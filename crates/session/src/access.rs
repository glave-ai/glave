//! A watch's caller's access, for as long as it follows something: a thread's watch, a turn's
//! stream, a list's watch. Each is let through once, as it begins; what can end the caller's
//! access while it runs is their browser's sign-in, ended here (signing out, signing other
//! browsers out, removing the passkey that signed it in, or a new sign-in past the most
//! kept), or their removal from the organization the watch is in. The hub tells every watch when one may have ended, and each that follows a browser's
//! session checks its own, so the watch ends at once rather than when its connection closes.
//! Projects have one owner, who keeps theirs: removing a project archives its threads, still
//! theirs to read.

use std::future::pending;
use std::sync::{Arc, Mutex, PoisonError};

use glave_event::PrincipalId;
use glave_identity::signin::{BrowserError, Browsers};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, OrgId};
use tokio::sync::watch;

use crate::browsers::session_of;
use crate::people::Peoples;
use crate::protocol::{ApiError, code};
use crate::server::Server;
use crate::words::Sentence;

/// What a watch's caller's access follows, and when to look at it again: the browser session
/// they signed in with, and their place in the organization the watch is in.
pub(crate) struct Access {
    /// Their browsers, and the session of theirs it follows, when they signed in with one.
    session: Option<(Arc<Browsers>, String)>,
    /// Who they are in which organization, and the people ledgers this server has read.
    member: Option<(PrincipalId, OrgId, Arc<Mutex<Peoples>>)>,
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
        if let Some((browsers, session)) = &self.session
            && !browsers.signed_in(session)
        {
            return Some(ApiError::from(BrowserError::Ended));
        }
        let (person, org, peoples) = self.member.as_ref()?;
        let peoples = peoples.lock().unwrap_or_else(PoisonError::into_inner);
        peoples
            .removed(*org, *person)
            .then(|| ApiError::new(code::NO_SIGN_IN, Sentence::Removed))
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
    /// with one here, and their place in its organization, for anyone but the local person;
    /// `None` for the local person over a local transport, whose access nothing here ends.
    /// Refused when it has ended already, checked after hearing begins so no ending falls
    /// between.
    pub(crate) async fn access(&self, caller: &Caller) -> Result<Option<Access>, ApiError> {
        let session = match session_of(caller) {
            // The browsers it signed in with: the local person's, or their glave account's.
            Some(session) => self
                .person_s(caller)
                .await
                .ok()
                .map(|browsers| (browsers, session.to_owned())),
            None => None,
        };
        let peoples = Arc::clone(&self.inner.people);
        let member = caller
            .person
            .map(|person| (person, self.org_of(caller), peoples));
        if session.is_none() && member.is_none() {
            return Ok(None);
        }
        let access = Access {
            session,
            member,
            sign_ins: self.inner.hub.sign_ins(),
            heard: false,
        };
        match access.refused() {
            Some(refused) => Err(refused),
            None => Ok(Some(access)),
        }
    }
}
