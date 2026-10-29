//! glave accounts for many people on one server (`specs/model.md`, "Identity"): each person's
//! passkeys and browser sessions in their account's own ledger, outside every organization
//! (`Browsers::open_account`), and the directory leading from a passkey's user handle to the
//! person. Each account's handle is derived from the server's secret and the person's id, so it
//! is random, says nothing of them, and needs no record. A browser signs in with any of its
//! passkeys for the public origin without saying whose first; its session's token names the
//! account, so any server on the store finds it.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use glave_event::PrincipalId;
use glave_ports::host::PublicOrigin;
use glave_ports::{BoxFuture, Clock, Directory, EventLog, IdentityKey, OrgId};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use super::browsers::{
    BrowserError, Browsers, Ceremony, Discovered, Discoverer, account_of, new_id,
};
use super::local::hex;

/// Accounts whose sessions and passkeys a server keeps in memory at once; past this, the
/// others are read again from the store when next used.
const OPEN: usize = 1_024;

/// How each account is opened from the store.
type Opener = Box<
    dyn Fn(PrincipalId, Uuid) -> BoxFuture<'static, Result<Browsers, BrowserError>> + Send + Sync,
>;

/// The glave accounts of one server, made at its public origin.
pub struct Accounts {
    open: Opener,
    directory: Arc<dyn Directory>,
    /// What each account's passkey handle is derived from, with its person's id.
    secret: [u8; 32],
    clock: Arc<dyn Clock>,
    opened: Mutex<BTreeMap<PrincipalId, Arc<Browsers>>>,
    discoverer: Discoverer,
}

/// A signed-in account, as a session's token proves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signed {
    pub person: PrincipalId,
    /// The session's id.
    pub session: String,
    /// The organizations the person belongs to, the first joined first.
    pub orgs: Vec<OrgId>,
}

impl Accounts {
    /// The accounts kept in `log`, found through `directory`, signed in by the server whose id
    /// is `server` at `origin`, their handles derived from `secret`.
    pub fn new<L: EventLog + Clone + 'static>(
        log: L,
        directory: Arc<dyn Directory>,
        secret: [u8; 32],
        server: String,
        origin: &PublicOrigin,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, BrowserError> {
        let (opening, at, ticking) = (log, origin.clone(), Arc::clone(&clock));
        let open: Opener = Box::new(move |person, handle| {
            let (log, server, origin) = (opening.clone(), server.clone(), at.clone());
            let clock: Box<dyn Clock> = Box::new(Arc::clone(&ticking));
            Box::pin(async move {
                Browsers::open_account(log, person, server, handle, &origin, clock).await
            })
        });
        Ok(Self {
            open,
            directory,
            secret,
            clock,
            opened: Mutex::new(BTreeMap::new()),
            discoverer: Discoverer::new(origin)?,
        })
    }

    /// `person`'s passkey handle: the start of a SHA-256 of the secret and their id.
    fn handle(&self, person: PrincipalId) -> Uuid {
        let mut hash = Sha256::new();
        hash.update(b"glave/account-handle/v1");
        hash.update(self.secret);
        hash.update(person.as_uuid().as_bytes());
        let digest: [u8; 32] = hash.finalize().into();
        let bytes = digest.first_chunk::<16>().copied().unwrap_or_default();
        uuid::Builder::from_custom_bytes(bytes).into_uuid()
    }

    /// The directory's key for the passkey handle `handle`.
    fn handle_key(handle: Uuid) -> Result<IdentityKey, BrowserError> {
        let mut hash = Sha256::new();
        hash.update(b"glave/passkey-handle/v1");
        hash.update(handle.as_bytes());
        IdentityKey::new(&hex(&hash.finalize())).map_err(|e| BrowserError::Keep(e.to_string()))
    }

    /// The sessions and passkeys of `person`'s account.
    pub async fn browsers(&self, person: PrincipalId) -> Result<Arc<Browsers>, BrowserError> {
        if let Some(open) = self.held().get(&person) {
            return Ok(Arc::clone(open));
        }
        let browsers = Arc::new((self.open)(person, self.handle(person)).await?);
        let mut held = self.held();
        if held.len() >= OPEN {
            held.clear();
        }
        Ok(Arc::clone(held.entry(person).or_insert(browsers)))
    }

    /// A new account: its person's id, and its sessions and passkeys, none yet. Nothing is
    /// kept of it until its first passkey is made and [`Accounts::found_by_its_passkeys`] is
    /// called.
    pub async fn made(&self) -> Result<(PrincipalId, Arc<Browsers>), BrowserError> {
        let id = new_id(self.clock.now())?;
        let person = PrincipalId::try_from(id).map_err(|e| BrowserError::Keep(e.to_string()))?;
        Ok((person, self.browsers(person).await?))
    }

    /// Links `person`'s passkey handle to them in the directory, so their passkeys sign them in.
    pub async fn found_by_its_passkeys(&self, person: PrincipalId) -> Result<(), BrowserError> {
        let key = Self::handle_key(self.handle(person))?;
        self.directory
            .link(&key, person)
            .await
            .map(drop)
            .map_err(|e| BrowserError::Keep(e.to_string()))
    }

    /// Records that `person` belongs to `org`, after the organizations they joined before.
    pub async fn join(&self, person: PrincipalId, org: OrgId) -> Result<(), BrowserError> {
        self.directory
            .join(person, org)
            .await
            .map_err(|e| BrowserError::Keep(e.to_string()))
    }

    /// Begins signing a browser in with any passkey made at the public origin.
    pub fn get_options(&self) -> Result<Ceremony, BrowserError> {
        self.discoverer.options(self.clock.now())
    }

    /// Whose passkey answered the request `ceremony` with `credential`, by its handle.
    pub fn identify(
        &self,
        ceremony: &str,
        credential: &serde_json::Value,
    ) -> Result<Discovered, BrowserError> {
        self.discoverer
            .identify(ceremony, credential, self.clock.now())
    }

    /// The account whose passkey answered as `discovered` says, signing a browser in with it:
    /// [`BrowserError::NoPasskey`] when no account has its handle.
    pub async fn signed_in_by(
        &self,
        discovered: Discovered,
    ) -> Result<(PrincipalId, super::Session), BrowserError> {
        let key = Self::handle_key(discovered.handle())?;
        let person = self
            .directory
            .find(&key)
            .await
            .map_err(|e| BrowserError::Keep(e.to_string()))?
            .ok_or(BrowserError::NoPasskey)?;
        let session = self
            .browsers(person)
            .await?
            .get_discovered(discovered)
            .await?;
        Ok((person, session))
    }

    /// The account a session's `token` proves, with the organizations its person belongs to.
    pub(crate) async fn verify(&self, token: &str, here: bool) -> Result<Signed, BrowserError> {
        let person = account_of(token).ok_or(BrowserError::Ended)?;
        let session = self.browsers(person).await?.verify(token, here).await?;
        let orgs = self
            .directory
            .organizations(person)
            .await
            .map_err(|e| BrowserError::Keep(e.to_string()))?;
        Ok(Signed {
            person,
            session,
            orgs,
        })
    }

    fn held(&self) -> std::sync::MutexGuard<'_, BTreeMap<PrincipalId, Arc<Browsers>>> {
        self.opened.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
