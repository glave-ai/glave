//! Signing up (`specs/model.md`, "Identity"), on a server whose configuration takes sign-ups
//! (`[serve] signup`), keeps glave accounts at a public origin and sends email: someone gives
//! their email and is sent a one-time link, good for 15 minutes; opening it makes their first
//! passkey, and with it their account and a new organization, theirs, with them its Owner.
//! The answer to an address is the same whether or not an account has it, so it says nothing
//! of who is here. Waiting sign-ups are kept in the server's own organization's sign-ups
//! ledger as the SHA-256 of their codes, so every server of a cell opens each link once.

use std::collections::BTreeMap;
use std::sync::{MutexGuard, PoisonError};

use glave_event::{
    At, Body, EventId, Header, OrgRole, PersonJoined, Principal, PrincipalId, Profile, RawEvent,
    RoleSet, Secret, Seq, SignupFinished, SignupStarted, encode,
};
use glave_identity::signin::{Proof, kept, one_time_code};
use glave_ports::stream::{Reserved, append_retrying, catch_up};
use glave_ports::{BlobStore, EventLog, Mail, ModelProvider, OrgId, Scope};
use jiff::{SignedDuration, Timestamp};
use uuid::Builder;

use crate::invites::gone;
use crate::ledger::in_order;
use crate::protocol::{ApiError, Reply, code};
use crate::server::Server;
use crate::words::{Language, Sentence, say};

/// How long a sign-up's link works: long enough to open it from a mailbox, short enough that
/// one left there is no key for long.
const LIFETIME: SignedDuration = SignedDuration::from_mins(15);

/// Accounts being made by sign-up at once; a new one past this forgets the oldest. Each
/// passkey request expires on its own, as any does.
const MOST: usize = 16;

/// The sign-ups waiting, as far as this server has read its own organization's ledger.
#[derive(Debug, Default)]
pub(crate) struct Signups {
    next: u64,
    /// By their codes' hashes: where each link was sent, and when it stops working.
    waiting: BTreeMap<String, SignupStarted>,
}

impl Signups {
    fn apply(&mut self, raw: RawEvent) {
        let Some((_, body)) = in_order(&mut self.next, raw) else {
            return;
        };
        match body {
            Body::SignupStarted(started) => {
                self.waiting.insert(started.hash.clone(), started);
            }
            Body::SignupFinished(finished) => {
                self.waiting.remove(&finished.hash);
            }
            _ => {}
        }
    }

    /// The sign-up whose code's hash is `hash`, while it works at `now`.
    fn opens(&self, hash: &str, now: Timestamp) -> Option<&SignupStarted> {
        self.waiting
            .get(hash)
            .filter(|started| now < started.expires.timestamp())
    }
}

/// An account being made by a sign-up's link.
pub(crate) struct Signing {
    ceremony: String,
    person: PrincipalId,
    hash: String,
    email: String,
}

/// The accounts being made by sign-up on this server.
#[derive(Default)]
pub(crate) struct Signings(Vec<Signing>);

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Whether this server takes sign-ups: its configuration says so, and it keeps glave
    /// accounts at a public origin and sends email.
    pub(crate) fn takes_signups(&self) -> bool {
        self.inner.signup
            && self.inner.accounts.is_some()
            && self.inner.mail.is_some()
            && self.inner.origin.is_some()
    }

    /// Answers `signup/start`: a sign-up's link emailed to `email`, unless an account has that
    /// email, or one went there a moment ago. The answer is the same either way.
    pub(crate) async fn start_signup(&self, email: &str) -> Result<Reply, ApiError> {
        if !self.takes_signups() {
            return Err(ApiError::new(code::NOT_AVAILABLE, Sentence::NoSignups));
        }
        let accounts = self.accounts()?;
        let (mailer, origin, to) = self.sender(email.trim())?;
        if accounts.with_email(to).await?.is_some() || !self.may_mail(to).await? {
            return Ok(Reply::Done {});
        }
        let code = one_time_code()?;
        let now = self.inner.harness.clock().now();
        let expires = now
            .checked_add(LIFETIME)
            .map_err(|e| ApiError::new(code::INTERNAL, e.to_string()))?;
        let started = SignupStarted {
            hash: kept(code.expose()),
            email: to.to_lowercase(),
            expires: At::from(expires),
        };
        self.record_signups(|_| Ok(Some(started.clone()))).await?;
        let link = format!("{origin}/#signup={}", code.expose());
        let mail = Mail {
            to: to.to_owned(),
            subject: say(Language::English, &Sentence::SignupMailSubject),
            text: format!(
                "{}\n\n{link}\n\n{}\n",
                say(Language::English, &Sentence::SignupMail),
                say(Language::English, &Sentence::SignupMailIgnore),
            ),
        };
        self.send(mailer, &mail).await?;
        Ok(Reply::Done {})
    }

    /// Answers `passkey/create_options` with a sign-up's `code`: begins making the first passkey
    /// of a new account, while the code opens a sign-up.
    pub(crate) async fn signup_options(&self, code: &Secret) -> Result<Reply, ApiError> {
        if !self.takes_signups() {
            return Err(gone());
        }
        let accounts = self.accounts()?;
        let hash = kept(code.expose());
        let now = self.inner.harness.clock().now();
        self.catch_up_signups().await?;
        let email = self
            .signups(|signups| signups.opens(&hash, now).map(|it| it.email.clone()))
            .ok_or_else(gone)?;
        let (person, browsers) = accounts.made().await?;
        let ceremony = browsers.create_options(Proof::SignedIn)?;
        let mut signing = self.signings();
        if signing.0.len() >= MOST {
            signing.0.remove(0);
        }
        signing.0.push(Signing {
            ceremony: ceremony.id.clone(),
            person,
            hash,
            email,
        });
        Ok(crate::browsers::options(ceremony))
    }

    /// The account being made by sign-up with the passkey request `ceremony`, taken, if one is.
    pub(crate) fn signing(&self, ceremony: &str) -> Option<Signing> {
        let mut signing = self.signings();
        let at = signing.0.iter().position(|it| it.ceremony == ceremony)?;
        Some(signing.0.remove(at))
    }

    /// Answers `passkey/create` for an account being made by sign-up: its first passkey, the
    /// sign-up spent, a new organization with the person its Owner, and the browser signed in.
    pub(crate) async fn finish_signup(
        &self,
        signing: Signing,
        credential: &serde_json::Value,
        name: Option<&str>,
    ) -> Result<Reply, ApiError> {
        let accounts = self.accounts()?;
        let browsers = accounts.browsers(signing.person).await?;
        // Checked, then the sign-up spent, and only then is anything of the account kept: a
        // link spent meanwhile leaves nothing behind.
        let checked = browsers.checked(&signing.ceremony, credential, name)?;
        let now = self.inner.harness.clock().now();
        self.record_signups(|signups| {
            signups.opens(&signing.hash, now).ok_or_else(gone)?;
            Ok(Some(SignupFinished {
                hash: signing.hash.clone(),
            }))
        })
        .await?;
        let session = browsers.keep(checked).await?;
        let millis = u64::try_from(now.as_millisecond()).unwrap_or_default();
        let random = self.inner.harness.entropy().bytes()?;
        let org =
            OrgId::from_uuid(Builder::from_unix_timestamp_millis(millis, &random).into_uuid());
        self.found(org, signing.person, &signing.email).await?;
        accounts.join(signing.person, org).await?;
        accounts.claim_email(&signing.email, signing.person).await?;
        Ok(self.signed_in(session, &browsers))
    }

    /// Records `person`, with `email`, as the first of the new organization `org`, its Owner,
    /// as glave makes them.
    async fn found(&self, org: OrgId, person: PrincipalId, email: &str) -> Result<(), ApiError> {
        let joined = PersonJoined {
            person,
            issuer: None,
            subject: None,
            profile: Profile {
                email: Some(email.to_owned()),
                ..Profile::default()
            },
        };
        let owner = RoleSet {
            person,
            role: OrgRole::Owner,
            by: Principal::glave(),
        };
        self.record_people(org, |_, next| {
            let after = next.saturating_add(1);
            Ok(Some(vec![
                self.people_event(next, &joined)?,
                self.people_event(after, &owner)?,
            ]))
        })
        .await
    }

    /// Reads whatever was added to the sign-ups ledger since this server last looked.
    async fn catch_up_signups(&self) -> Result<(), ApiError> {
        // Glave's own: the server's own organization's sign-ups ledger, read only here.
        let scope = Scope::of_organization(self.inner.own);
        let next = self.signups(|signups| signups.next.max(1));
        let after = Seq::new(next.saturating_sub(1)).ok();
        let log = self.inner.harness.log();
        catch_up(log, &scope, Reserved::Signups.id(), after, |page| {
            self.signups(|held| {
                for raw in page {
                    held.apply(raw);
                }
            });
        })
        .await?;
        Ok(())
    }

    /// Appends what `make` makes of the sign-ups as they are then, after anything others wrote
    /// meanwhile: nothing when it makes nothing.
    async fn record_signups<T: glave_event::Payload>(
        &self,
        make: impl Fn(&Signups) -> Result<Option<T>, ApiError>,
    ) -> Result<(), ApiError> {
        // Glave's own: the server's own organization's sign-ups ledger.
        let scope = Scope::of_organization(self.inner.own);
        let ledger = Reserved::Signups.id();
        let log = self.inner.harness.log();
        let make = &make;
        let appended = append_retrying(log, &scope, ledger, || async move {
            self.catch_up_signups().await?;
            let made = self.signups(|held| make(held))?;
            let Some(payload) = made else {
                return Ok::<_, ApiError>(None);
            };
            let seq = self.signups(|held| held.next.max(1));
            let now = self.inner.harness.clock().now();
            let header = Header {
                id: EventId::new(now, self.inner.harness.entropy().bytes()?)?,
                thread: ledger,
                seq: Seq::new(seq)?,
                at: At::from(now),
            };
            Ok(Some((vec![encode(header, &payload)?], ())))
        })
        .await?;
        self.signups(|held| {
            for raw in appended.into_iter().flat_map(|(events, ())| events) {
                held.apply(raw);
            }
        });
        Ok(())
    }

    fn signups<R>(&self, look: impl FnOnce(&mut Signups) -> R) -> R {
        let mut held = self
            .inner
            .signups
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        look(&mut held)
    }

    fn signings(&self) -> MutexGuard<'_, Signings> {
        self.inner
            .signings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}
