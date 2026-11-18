//! A watch's caller's access, for as long as it follows something: a thread's watch, a turn's
//! stream, a list's watch. Each is let through once, as it begins; what can end the caller's
//! access while it runs is their browser's sign-in, ended here (signing out, signing other
//! browsers out, removing the passkey that signed it in, or a new sign-in past the most
//! kept), their company login's token expiring, or their removal from the organization the
//! watch is in. The hub tells every watch when one may have ended, and each checks its own; a
//! token's expiry is looked at before anything is sent, and an idle watch wakes for it; so the
//! watch ends at once rather than when its connection closes.
//! Projects have one owner, who keeps theirs: removing a project archives its threads, still
//! theirs to read.

use std::future::pending;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use glave_event::PrincipalId;
use glave_identity::signin::{BrowserError, Browsers};
use glave_ports::{BlobStore, Caller, Clock, EventLog, ModelProvider, OrgId};
use jiff::Timestamp;
use tokio::sync::watch;

use crate::browsers::session_of;
use crate::people::Peoples;
use crate::protocol::{ApiError, code};
use crate::server::Server;
use crate::words::Sentence;

/// What a watch's caller's access follows, and when to look at it again: the browser session
/// they signed in with, when their sign-in stops being taken, and their place in the
/// organization the watch is in.
pub(crate) struct Access {
    /// Their browsers, and the session of theirs it follows, when they signed in with one.
    session: Option<(Arc<Browsers>, String)>,
    /// When their sign-in stops being taken, a company login's token, and the clock it is
    /// read on.
    until: Option<(Timestamp, Arc<dyn Clock>)>,
    /// Who they are, and their place in which organization.
    member: Option<(PrincipalId, Following)>,
    sign_ins: watch::Receiver<()>,
    /// Whether a sign-in may have ended since the session was last looked at: waiting for
    /// the change takes it as seen.
    heard: bool,
}

impl Access {
    /// Waits until a sign-in here may have ended, or the caller's token expires, for a watch
    /// whose caller's `access` something here can end; never for any other, nor once the
    /// server has gone.
    pub(crate) async fn changed(access: &mut Option<Self>) {
        let heard = match access {
            Some(access) => {
                let left = access.left();
                access.heard = tokio::select! {
                    changed = access.sign_ins.changed() => changed.is_ok(),
                    // Looked at once the loop comes round, which finds the token expired.
                    () = expires(left) => true,
                };
                access.heard
            }
            None => false,
        };
        if !heard {
            pending::<()>().await;
        }
    }

    /// Why the caller may no longer follow what they watch: their token has expired, or a
    /// sign-in here may have ended and theirs has.
    pub(crate) fn ended(&mut self) -> Option<ApiError> {
        let heard = std::mem::take(&mut self.heard) || self.sign_ins.has_changed().unwrap_or(false);
        if !heard {
            // Whatever else was heard, so nothing is sent once the token has expired.
            return self.expired();
        }
        self.sign_ins.mark_unchanged();
        self.refused()
    }

    fn refused(&self) -> Option<ApiError> {
        if let Some(expired) = self.expired() {
            return Some(expired);
        }
        if let Some((browsers, session)) = &self.session
            && !browsers.signed_in(session)
        {
            return Some(ApiError::from(BrowserError::Ended));
        }
        let (person, place) = self.member.as_ref()?;
        place
            .removed(*person)
            .then(|| ApiError::new(code::NO_SIGN_IN, Sentence::Removed))
    }

    /// Why the caller may no longer follow anything, once the clock has reached when their
    /// sign-in stops being taken.
    fn expired(&self) -> Option<ApiError> {
        let (until, clock) = self.until.as_ref()?;
        (clock.now() >= *until).then(|| ApiError::new(code::NO_SIGN_IN, Sentence::TokenExpired))
    }

    /// How long until the caller's sign-in stops being taken, for one that does.
    fn left(&self) -> Option<Duration> {
        let (until, clock) = self.until.as_ref()?;
        Some(Duration::try_from(until.duration_since(clock.now())).unwrap_or_default())
    }
}

/// Waits `left`, or for ever when nothing runs out.
async fn expires(left: Option<Duration>) {
    match left {
        Some(left) => tokio::time::sleep(left).await,
        None => pending().await,
    }
}

/// A watch's hold on an organization's people ledger, which this server reads again while
/// any watch holds it, so a removal on another server on the store reaches the watch.
struct Following {
    org: OrgId,
    peoples: Arc<Mutex<Peoples>>,
}

impl Following {
    fn new(org: OrgId, peoples: Arc<Mutex<Peoples>>) -> Self {
        peoples
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .watch(org);
        Self { org, peoples }
    }

    fn removed(&self, person: PrincipalId) -> bool {
        self.peoples
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .removed(self.org, person)
    }
}

impl Drop for Following {
    fn drop(&mut self) {
        self.peoples
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .unwatch(self.org);
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
    /// with one here, when their sign-in stops being taken, and their place in its
    /// organization, for anyone but the local person; `None` for the local person over a local
    /// transport, whose access nothing here ends.
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
        let org = self.org_of(caller);
        // The person on the server's computer is never removed from its own organization.
        let member = caller
            .person
            .filter(|person| self.refuse_local(org, *person).is_ok())
            .map(|person| {
                let peoples = Arc::clone(&self.inner.people);
                (person, Following::new(org, peoples))
            });
        let until = caller
            .until
            .map(|until| (until, Arc::clone(&self.inner.clock)));
        if session.is_none() && until.is_none() && member.is_none() {
            return Ok(None);
        }
        let access = Access {
            session,
            until,
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
