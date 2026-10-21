//! Signing browsers in on the person's own computer (`specs/one-server.md`, "Finding it"): a
//! program here asks for a link's one-time code, the browser trades it for a session of its
//! own, and ends it, or every other browser's, when the person signs out. Where the server is
//! reached from elsewhere, a browser there signs in with a passkey, the first made through a
//! setup link from a program here (`specs/session.md`, "Signing in"). Sessions are only ever
//! the local person's: nobody an identity provider signs in gets one, so a browser's sign-in
//! through a provider ends when the provider stops renewing it.

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
        if browsers.offers_passkeys() {
            ways.push(SignInWay::Passkey);
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
        let browsers = self.browsers()?;
        let (_, session) = browsers.create(ceremony, credential, name).await?;
        Ok(signed_in(session, browsers))
    }

    /// Answers `passkey/add_options`: adding a passkey for the person signed in.
    pub(crate) fn add_options(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let browsers = self.person_s(caller)?;
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
        let browsers = self.person_s(caller)?;
        let passkey = browsers.add(ceremony, credential, name).await?;
        Ok(Reply::Passkey {
            passkey: summary(passkey),
        })
    }

    /// Answers `passkey/get_options`: signing a browser in with a passkey.
    pub(crate) fn get_options(&self) -> Result<Reply, ApiError> {
        Ok(options(self.browsers()?.get_options()?))
    }

    /// Answers `passkey/get`: the browser signed in with a passkey.
    pub(crate) async fn get(
        &self,
        ceremony: &str,
        credential: &serde_json::Value,
    ) -> Result<Reply, ApiError> {
        let browsers = self.browsers()?;
        let session = browsers.get(ceremony, credential).await?;
        Ok(signed_in(session, browsers))
    }

    /// Answers `passkey/list`: the person's passkeys.
    pub(crate) fn passkeys(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let browsers = self.person_s(caller)?;
        let passkeys = browsers.passkeys().into_iter().map(summary).collect();
        Ok(Reply::Passkeys { passkeys })
    }

    /// Answers `passkey/remove`.
    pub(crate) async fn remove_passkey(
        &self,
        caller: &Caller,
        passkey: &str,
    ) -> Result<Reply, ApiError> {
        self.person_s(caller)?.remove(passkey).await?;
        Ok(Reply::Done {})
    }

    /// The browsers, for the person on this computer, signed in.
    fn person_s(&self, caller: &Caller) -> Result<&Browsers, ApiError> {
        let browsers = self.browsers()?;
        if !local(caller) {
            return Err(ApiError::new(
                code::NOT_ALLOWED,
                Sentence::PasskeysOnlyTheirs,
            ));
        }
        Ok(browsers)
    }

    /// Answers `signin/redeem`: `code`, once, for a session with this server.
    pub(crate) async fn redeem(&self, code: &str) -> Result<Reply, ApiError> {
        let browsers = self.browsers()?;
        let session = browsers.redeem(code).await?;
        Ok(Reply::Browser {
            session: session.token,
            server: browsers.server(),
        })
    }

    /// Answers `signin/end`: ends the caller's own browser session.
    pub(crate) async fn end_session(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let browsers = self.browsers()?;
        let session = session_of(caller)
            .ok_or_else(|| ApiError::new(code::INVALID_PARAMS, Sentence::NotABrowser))?;
        browsers.end(session).await?;
        Ok(Reply::Done {})
    }

    /// Answers `signin/end_others`: ends every browser session but the caller's own, for the
    /// person on this computer.
    pub(crate) async fn end_others(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let browsers = self.browsers()?;
        if !local(caller) {
            return Err(ApiError::new(code::NOT_ALLOWED, Sentence::SignOutOnlyHere));
        }
        browsers.end_others(session_of(caller)).await?;
        Ok(Reply::Done {})
    }

    fn browsers(&self) -> Result<&Browsers, ApiError> {
        self.inner
            .browsers
            .as_deref()
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
fn options(ceremony: Ceremony) -> Reply {
    Reply::PasskeyOptions {
        ceremony: ceremony.id,
        options: ceremony.options,
    }
}

/// `session`, begun with this server of `browsers`.
fn signed_in(session: Session, browsers: &Browsers) -> Reply {
    Reply::Browser {
        session: session.token,
        server: browsers.server(),
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

/// The browser session `caller` signed in with, if they did.
fn session_of(caller: &Caller) -> Option<&str> {
    caller
        .auth
        .as_ref()
        .filter(|auth| auth.method == AuthMethod::Local)
        .and_then(|auth| auth.session.as_deref())
}
