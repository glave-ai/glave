//! glave accounts for many people on one server (`specs/model.md`, "Identity"): each person's
//! passkeys and browser sessions in their account's own ledger, outside every organization
//! (`Browsers::open_account`), and the organizations they joined in the directory. An
//! account's passkeys keep its person's id as their user handle, so a passkey says whose
//! account to look in, and its signature is checked against that account's passkeys alone. A
//! browser signs in with any passkey for the public origin without saying whose first; its
//! session's token names the account, so any server on the store finds it.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use glave_event::PrincipalId;
use glave_ports::host::PublicOrigin;
use glave_ports::{BoxFuture, Clock, Directory, EventLog, IdentityKey, OrgId};

use super::browsers::{
    BrowserError, Browsers, Ceremony, Discovered, Discoverer, Session, account_of, kept, new_id,
};

/// Accounts whose sessions and passkeys a server keeps in memory at once. Past this, those no
/// watch or request holds are forgotten, read again from the store when next used.
const OPEN: usize = 1_024;

/// How each account is opened from the store.
type Opener =
    Box<dyn Fn(PrincipalId) -> BoxFuture<'static, Result<Browsers, BrowserError>> + Send + Sync>;

/// The glave accounts of one server, made at its public origin.
pub struct Accounts {
    open: Opener,
    directory: Arc<dyn Directory>,
    clock: Arc<dyn Clock>,
    opened: Mutex<BTreeMap<PrincipalId, Arc<Browsers>>>,
    discoverer: Discoverer,
}

/// A signed-in account, as a session's token proves it.
pub(crate) struct Signed {
    pub(crate) person: PrincipalId,
    /// The session's id.
    pub(crate) session: String,
    /// The organizations the person belongs to, the first joined first.
    pub(crate) orgs: Vec<OrgId>,
}

impl Accounts {
    /// The accounts kept in `log`, their organizations in `directory`, signed in by the server
    /// whose id is `server` at `origin`.
    pub fn new<L: EventLog + Clone + 'static>(
        log: L,
        directory: Arc<dyn Directory>,
        server: String,
        origin: &PublicOrigin,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, BrowserError> {
        let (opening, at, ticking) = (log, origin.clone(), Arc::clone(&clock));
        let open: Opener = Box::new(move |person| {
            let (log, server, origin) = (opening.clone(), server.clone(), at.clone());
            let clock: Box<dyn Clock> = Box::new(Arc::clone(&ticking));
            Box::pin(async move {
                let handle = person.as_uuid();
                Browsers::open_account(log, person, server, handle, &origin, clock).await
            })
        });
        Ok(Self {
            open,
            directory,
            clock,
            opened: Mutex::new(BTreeMap::new()),
            discoverer: Discoverer::new(origin)?,
        })
    }

    /// The sessions and passkeys of `person`'s account.
    pub async fn browsers(&self, person: PrincipalId) -> Result<Arc<Browsers>, BrowserError> {
        if let Some(open) = self.held().get(&person) {
            return Ok(Arc::clone(open));
        }
        let browsers = Arc::new((self.open)(person).await?);
        let mut held = self.held();
        if held.len() >= OPEN {
            // One a watch or a request still holds is kept, so what ends its sessions reaches
            // the one it follows.
            held.retain(|_, open| Arc::strong_count(open) > 1);
        }
        if held.len() >= OPEN {
            return Ok(browsers);
        }
        Ok(Arc::clone(held.entry(person).or_insert(browsers)))
    }

    /// The sessions and passkeys of `person`'s account, when they belong to an organization
    /// here: the organizations, the first joined first.
    async fn member(
        &self,
        person: PrincipalId,
    ) -> Result<(Arc<Browsers>, Vec<OrgId>), BrowserError> {
        let orgs = self.organizations(person).await?;
        if orgs.is_empty() {
            return Err(BrowserError::Ended);
        }
        Ok((self.browsers(person).await?, orgs))
    }

    /// A new account: its person's id, and its sessions and passkeys, none yet. Nothing is
    /// kept of it until its first passkey is.
    pub async fn made(&self) -> Result<(PrincipalId, Arc<Browsers>), BrowserError> {
        let id = new_id(self.clock.now())?;
        let person = PrincipalId::try_from(id).map_err(|e| BrowserError::Keep(e.to_string()))?;
        Ok((person, self.browsers(person).await?))
    }

    /// The organizations `person` belongs to, the first joined first.
    pub async fn organizations(&self, person: PrincipalId) -> Result<Vec<OrgId>, BrowserError> {
        self.directory
            .organizations(person)
            .await
            .map_err(|e| BrowserError::Keep(e.to_string()))
    }

    /// The account whose email is `address`, if one has it, as [`Accounts::claim_email`]
    /// gave it.
    pub async fn with_email(&self, address: &str) -> Result<Option<PrincipalId>, BrowserError> {
        self.directory
            .find(&email_key(address)?)
            .await
            .map_err(|e| BrowserError::Keep(e.to_string()))
    }

    /// Gives `person`'s account the email `address`, for getting back in, unless another
    /// account has it already: whose it is now.
    pub async fn claim_email(
        &self,
        address: &str,
        person: PrincipalId,
    ) -> Result<PrincipalId, BrowserError> {
        self.directory
            .link(&email_key(address)?, person)
            .await
            .map_err(|e| BrowserError::Keep(e.to_string()))
    }

    /// Records that `person` belongs to `org`, after the organizations they joined before.
    pub async fn join(&self, person: PrincipalId, org: OrgId) -> Result<(), BrowserError> {
        self.directory
            .join(person, org)
            .await
            .map_err(|e| BrowserError::Keep(e.to_string()))
    }

    /// Takes `person` out of `org`: their sessions sign them in there no more.
    pub async fn leave(&self, person: PrincipalId, org: OrgId) -> Result<(), BrowserError> {
        self.directory
            .leave(person, org)
            .await
            .map_err(|e| BrowserError::Keep(e.to_string()))
    }

    /// Begins signing a browser in with any passkey made at the public origin.
    pub fn get_options(&self) -> Result<Ceremony, BrowserError> {
        self.discoverer.options(self.clock.now())
    }

    /// Whether the passkey request `ceremony` is one [`Accounts::get_options`] began.
    pub fn began(&self, ceremony: &str) -> bool {
        self.discoverer.waits(ceremony)
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

    /// Signs a browser in with the account passkey `discovered` says it is, its handle its
    /// person's id: [`BrowserError::NoPasskey`] for one that is no member's here.
    pub async fn signed_in_by(
        &self,
        discovered: Discovered,
    ) -> Result<(PrincipalId, Session), BrowserError> {
        let person =
            PrincipalId::try_from(discovered.handle()).map_err(|_| BrowserError::NoPasskey)?;
        let (browsers, _) = self
            .member(person)
            .await
            .map_err(|_| BrowserError::NoPasskey)?;
        let session = browsers.get_discovered(discovered).await?;
        Ok((person, session))
    }

    /// The account a session's `token` proves, with the organizations its person belongs to;
    /// one in none is no longer signed in.
    pub(crate) async fn verify(&self, token: &str, here: bool) -> Result<Signed, BrowserError> {
        let person = account_of(token).ok_or(BrowserError::Ended)?;
        let (browsers, orgs) = self.member(person).await?;
        let session = browsers.verify(token, here).await?;
        Ok(Signed {
            person,
            session,
            orgs,
        })
    }

    fn held(&self) -> MutexGuard<'_, BTreeMap<PrincipalId, Arc<Browsers>>> {
        self.opened.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What an email address is kept as in the directory: the SHA-256 of the address as glave
/// reads it, lowercase and trimmed, apart from every other identity, so no address is kept.
fn email_key(address: &str) -> Result<IdentityKey, BrowserError> {
    let read = address.trim().to_lowercase();
    IdentityKey::new(&kept(&format!("email\0{read}")))
        .map_err(|e| BrowserError::Keep(e.to_string()))
}
