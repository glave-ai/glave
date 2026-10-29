//! Signing browsers in on the person's own computer (`specs/one-server.md`, "Finding it"): a
//! program here asks for a link's one-time code, the browser trades it for a session of its
//! own, and ends it, or every other browser's, when the person signs out. Where the server is
//! reached from elsewhere, a browser there signs in with a passkey, the first made through a
//! setup link from a program here; where it is not, a browser here signed in by a link makes
//! one, which signs any browser here in (`specs/session.md`, "Signing in"). Sessions are only
//! ever the local person's: nobody an identity provider signs in gets one, so a browser's
//! sign-in through a provider ends when the provider stops renewing it.

use std::sync::Arc;

use glave_event::{At, AuthMethod, Secret};
use glave_identity::signin::{Browsers, Ceremony, PasskeyInfo, Proof, Session};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};

use crate::protocol::{ApiError, PasskeySummary, Reply, SignInWay, code};
use crate::providers::local;
use crate::server::Server;
use crate::words::Sentence;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Where a browser on this computer makes and uses passkeys, when the server makes them
    /// here.
    pub(crate) fn passkeys_here(&self) -> Option<String> {
        self.inner
            .browsers
            .as_ref()
            .and_then(|browsers| browsers.passkeys_here())
    }

    /// The server's id, where it signs browsers in.
    pub(crate) fn server_id(&self) -> Option<String> {
        self.inner
            .browsers
            .as_ref()
            .map(|browsers| browsers.server())
    }

    /// How a browser signs in to this server, for `server/discover`.
    pub(crate) fn signin_ways(&self) -> Vec<SignInWay> {
        let Some(browsers) = self.inner.browsers.as_deref() else {
            return Vec::new();
        };
        let mut ways = vec![SignInWay::Link];
        if browsers.signs_in_with_passkeys() || self.takes_signups() {
            ways.push(SignInWay::Passkey);
        }
        if self.takes_signups() {
            ways.push(SignInWay::Signup);
        }
        ways
    }

    /// Answers `signin/link`: a code for the person on this computer, asked for by a program
    /// signed in as them, never by a browser with a session of its own.
    pub(crate) fn link(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let browsers = self.browsers()?;
        by_program(caller)?;
        let code = browsers.link()?;
        Ok(Reply::Link { code })
    }

    /// Answers `signin/setup`: a setup link's code, asked for by a program on this computer.
    pub(crate) fn setup(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let browsers = self.browsers()?;
        by_program(caller)?;
        let code = browsers.setup()?;
        Ok(Reply::Link { code })
    }

    /// Answers `passkey/create_options`: making a passkey with a setup link's `code`.
    pub(crate) fn create_options(&self, code: &Secret) -> Result<Reply, ApiError> {
        let ceremony = self
            .browsers()?
            .create_options(Proof::Code(code.expose()))?;
        Ok(options(ceremony))
    }

    /// Answers `passkey/create`: the passkey a setup link made, and the browser signed in.
    pub(crate) async fn create(
        &self,
        ceremony: &str,
        credential: &serde_json::Value,
        name: Option<&str>,
    ) -> Result<Reply, ApiError> {
        if let Some(joining) = self.joining(ceremony) {
            return self.join(joining, credential, name).await;
        }
        if let Some(signing) = self.signing(ceremony) {
            return self.finish_signup(signing, credential, name).await;
        }
        let browsers = self.browsers()?;
        let (_, session) = browsers.create(ceremony, credential, name).await?;
        Ok(self.signed_in(session, browsers))
    }

    /// Answers `passkey/add_options`: adding a passkey for the person signed in.
    pub(crate) async fn add_options(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let browsers = self.person_s(caller).await?;
        Ok(options(browsers.create_options(Proof::SignedIn)?))
    }

    /// Answers `passkey/add`: the passkey the person signed in added.
    pub(crate) async fn add(
        &self,
        caller: &Caller,
        ceremony: &str,
        credential: &serde_json::Value,
        name: Option<&str>,
    ) -> Result<Reply, ApiError> {
        let browsers = self.person_s(caller).await?;
        let passkey = browsers.add(ceremony, credential, name).await?;
        Ok(Reply::Passkey {
            passkey: summary(passkey),
        })
    }

    /// Answers `passkey/get_options`: signing a browser in with a passkey. Where people other
    /// than the local person have glave accounts here, with any passkey for the public origin,
    /// whose it is told by the passkey; else with one of the local person's.
    pub(crate) async fn get_options(&self) -> Result<Reply, ApiError> {
        if let Some(accounts) = self.inner.accounts.as_deref() {
            // Where anyone may sign up, accounts are many and in organizations of their own.
            if self.takes_signups() {
                return Ok(options(accounts.get_options()?));
            }
            let own = self.inner.own;
            self.catch_up_people(own).await?;
            if self.people(own, |people| people.has_accounts()) {
                return Ok(options(accounts.get_options()?));
            }
        }
        Ok(options(self.browsers()?.get_options()?))
    }

    /// Answers `passkey/get`: the browser signed in with a passkey, the local person's or, by
    /// its handle, someone's glave account's.
    pub(crate) async fn get(
        &self,
        ceremony: &str,
        credential: &serde_json::Value,
    ) -> Result<Reply, ApiError> {
        let browsers = self.browsers()?;
        let accounts = self.inner.accounts.as_deref();
        let Some(accounts) = accounts.filter(|accounts| accounts.began(ceremony)) else {
            let session = browsers.get(ceremony, credential).await?;
            return Ok(self.signed_in(session, browsers));
        };
        let discovered = accounts.identify(ceremony, credential)?;
        if discovered.handle() == browsers.handle() {
            let session = browsers.get_discovered(discovered).await?;
            return Ok(self.signed_in(session, browsers));
        }
        let (_, session) = accounts.signed_in_by(discovered).await?;
        Ok(self.signed_in(session, browsers))
    }

    /// Answers `passkey/list`: the person's passkeys.
    pub(crate) async fn passkeys(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let browsers = self.person_s(caller).await?;
        let passkeys = browsers.passkeys().into_iter().map(summary).collect();
        Ok(Reply::Passkeys { passkeys })
    }

    /// Answers `passkey/remove`. A glave account keeps its last passkey, since nothing else
    /// signs its person in yet.
    pub(crate) async fn remove_passkey(
        &self,
        caller: &Caller,
        passkey: &str,
    ) -> Result<Reply, ApiError> {
        let browsers = self.person_s(caller).await?;
        let last = browsers.passkeys().iter().all(|kept| kept.id == passkey);
        if last && is_account(caller) {
            return Err(ApiError::new(code::NOT_ALLOWED, Sentence::LastPasskey));
        }
        let removed = browsers.remove(passkey).await;
        // The sessions it signed in end with it.
        self.inner.hub.sign_ins_changed();
        removed?;
        Ok(Reply::Done {})
    }

    /// The browsers of `caller`: the local person's, or their glave account's.
    pub(crate) async fn person_s(&self, caller: &Caller) -> Result<Arc<Browsers>, ApiError> {
        if let (true, Some(person)) = (is_account(caller), caller.person) {
            return Ok(self.accounts()?.browsers(person).await?);
        }
        if !local(caller) {
            return Err(ApiError::new(
                code::NOT_ALLOWED,
                Sentence::PasskeysOnlyTheirs,
            ));
        }
        self.browsers().map(Arc::clone)
    }

    /// Answers `signin/redeem`: `code`, once, for a session with this server.
    pub(crate) async fn redeem(&self, code: &str) -> Result<Reply, ApiError> {
        let browsers = self.browsers()?;
        let session = browsers.redeem(code).await?;
        Ok(self.signed_in(session, browsers))
    }

    /// Answers `signin/end`: ends the caller's own browser session.
    pub(crate) async fn end_session(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let session = session_of(caller)
            .ok_or_else(|| ApiError::new(code::INVALID_PARAMS, Sentence::NotABrowser))?;
        let browsers = self.person_s(caller).await?;
        let ended = browsers.end(session).await;
        self.inner.hub.sign_ins_changed();
        ended?;
        Ok(Reply::Done {})
    }

    /// Answers `signin/end_others`: ends every browser session but the caller's own, for the
    /// person on this computer.
    pub(crate) async fn end_others(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let browsers = self
            .person_s(caller)
            .await
            .map_err(|_| ApiError::new(code::NOT_ALLOWED, Sentence::SignOutOnlyHere))?;
        let ended = browsers.end_others(session_of(caller)).await;
        self.inner.hub.sign_ins_changed();
        ended?;
        Ok(Reply::Done {})
    }

    /// `session`, begun with `browsers`, as the browser is told of it. Beginning it may have
    /// ended the session used longest ago, past the most kept.
    pub(crate) fn signed_in(&self, session: Session, browsers: &Browsers) -> Reply {
        self.inner.hub.sign_ins_changed();
        Reply::Browser {
            session: session.token,
            server: browsers.server(),
            accepted: session.accepted,
        }
    }

    fn browsers(&self) -> Result<&Arc<Browsers>, ApiError> {
        self.inner
            .browsers
            .as_ref()
            .ok_or_else(|| ApiError::new(code::NOT_AVAILABLE, Sentence::NoBrowsers))
    }
}

/// Refuses `caller` unless they are a program on this computer signed in as the person: never
/// a browser with a session of its own.
fn by_program(caller: &Caller) -> Result<(), ApiError> {
    let by_program = caller
        .auth
        .as_ref()
        .is_none_or(|auth| auth.method == AuthMethod::Local && auth.session.is_none());
    if by_program {
        Ok(())
    } else {
        Err(ApiError::new(code::NOT_ALLOWED, Sentence::LinksOnlyHere))
    }
}

/// `ceremony`, as `passkey/*_options` answer it.
pub(crate) fn options(ceremony: Ceremony) -> Reply {
    Reply::PasskeyOptions {
        ceremony: ceremony.id,
        options: ceremony.options,
    }
}

/// `passkey` as a client lists it.
fn summary(passkey: PasskeyInfo) -> PasskeySummary {
    PasskeySummary {
        id: passkey.id,
        name: passkey.name,
        added: At::from(passkey.added),
        used: passkey.used.map(At::from),
    }
}

/// Whether `caller` signed in with a glave account.
fn is_account(caller: &Caller) -> bool {
    caller
        .auth
        .as_ref()
        .is_some_and(|auth| auth.method == AuthMethod::Account)
}

/// The browser session `caller` signed in with, if they did.
pub(crate) fn session_of(caller: &Caller) -> Option<&str> {
    caller
        .auth
        .as_ref()
        .filter(|auth| matches!(auth.method, AuthMethod::Local | AuthMethod::Account))
        .and_then(|auth| auth.session.as_deref())
}
