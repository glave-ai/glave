//! Signing up (`specs/model.md`, "Identity"), on a server whose configuration takes sign-ups
//! (`[serve] signup`), keeps glave accounts at a public origin and sends email: someone gives
//! their email and is sent a one-time link, good for 15 minutes; opening it makes their first
//! passkey, and with it their account and a new organization, theirs, with them its Owner.
//! An address that has an account is sent a note to sign in instead, through the same steps,
//! so the answer, and how long it takes, say nothing of who is here. The link's code carries
//! the address after its secret; waiting sign-ups are kept in the server's own organization's
//! sign-ups ledger as the SHA-256 of their codes and of their addresses alone, so every server
//! of a cell opens each link once, counts each address's messages, and keeps no address of
//! anyone who never finished.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
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

use crate::mailing::just_now;
use crate::protocol::{ApiError, Reply, code};
use crate::server::Server;
use crate::words::{Language, Sentence, say};
use glave_ports::stream::in_order;

/// How long a sign-up's link works: long enough to open it from a mailbox, short enough that
/// one left there is no key for long.
const LIFETIME: SignedDuration = SignedDuration::from_mins(15);

/// Accounts being made by sign-up at once; a new one past this forgets the oldest. Each
/// passkey request expires on its own, as any does.
const MOST: usize = 16;

/// Sign-ups waiting at once, at most; past this, an address is answered as ever and sent
/// nothing, until some expire.
const MOST_WAITING: usize = 10_000;

/// What parts a sign-up's code: its secret, then the address it was sent to, in hex.
const TO: char = '.';

/// The sign-ups waiting, as far as this server has read its own organization's ledger.
#[derive(Debug, Default)]
pub(crate) struct Signups {
    next: u64,
    /// By their codes' hashes: when each began, and where its link went and when it stops
    /// working, as hashes.
    waiting: BTreeMap<String, (Timestamp, SignupStarted)>,
}

impl Signups {
    fn apply(&mut self, raw: RawEvent) {
        let Some((at, body)) = in_order(&mut self.next, raw) else {
            return;
        };
        match body {
            Body::SignupStarted(started) => {
                self.waiting
                    .insert(started.hash.clone(), (at.timestamp(), started));
            }
            Body::SignupFinished(finished) => {
                self.waiting.remove(&finished.hash);
            }
            _ => {}
        }
    }

    /// Forgets those expired at `now`: nothing can open them.
    fn drop_expired(&mut self, now: Timestamp) {
        self.waiting
            .retain(|_, (_, started)| now < started.expires.timestamp());
    }

    /// The link of the kind `emailed` whose code's hash is `hash`, while it works at `now`.
    fn opens(&self, hash: &str, emailed: Emailed, now: Timestamp) -> Option<&SignupStarted> {
        self.waiting
            .get(hash)
            .map(|(_, started)| started)
            .filter(|started| now < started.expires.timestamp())
            .filter(|started| started.recovery == (emailed == Emailed::Recovery))
    }

    /// Whether a message may go to the address whose hash is `to` at `now`, and room is left.
    fn may_send(&self, to: &str, now: Timestamp) -> bool {
        self.waiting.len() < MOST_WAITING
            && !self
                .waiting
                .values()
                .any(|(at, started)| started.to == to && just_now(*at, now))
    }
}

/// What an emailed link is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Emailed {
    /// Signing up: a new account, and an organization of its own.
    Signup,
    /// Getting back in: a new passkey for the account the address has.
    Recovery,
}

/// A passkey being made by an emailed link: a new account's, or an account's to get back in.
pub(crate) struct Signing {
    ceremony: String,
    pub(crate) person: PrincipalId,
    pub(crate) hash: String,
    pub(crate) email: String,
    pub(crate) emailed: Emailed,
}

/// The passkeys being made by emailed links on this server.
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

    /// Answers `signup/start` and `recovery/start`: a link of the kind `emailed` sent to
    /// `email`, or, where the address has an account (for a sign-up) or none (to get back in,
    /// where the server takes sign-ups), a note saying so; nothing when a message went there a
    /// moment ago. The answer is the same either way, and so is what is recorded.
    pub(crate) async fn start_link(
        &self,
        email: &str,
        emailed: Emailed,
    ) -> Result<Reply, ApiError> {
        match emailed {
            Emailed::Signup if !self.takes_signups() => {
                return Err(ApiError::new(code::NOT_AVAILABLE, Sentence::NoSignups));
            }
            Emailed::Recovery if !self.recovers() => {
                return Err(ApiError::new(code::NOT_AVAILABLE, Sentence::NoRecovery));
            }
            Emailed::Signup | Emailed::Recovery => {}
        }
        let accounts = self.accounts()?;
        let (mailer, origin, to) = self.sender(email)?;
        let has_account = accounts.with_email(&to).await?.is_some();
        let secret = one_time_code()?;
        let code = format!("{}{TO}{}", secret.expose(), hex(to.as_bytes()));
        let now = self.inner.harness.clock().now();
        let expires = now
            .checked_add(LIFETIME)
            .map_err(|e| ApiError::new(code::INTERNAL, e.to_string()))?;
        let started = SignupStarted {
            hash: kept(&code),
            to: kept(&to),
            expires: At::from(expires),
            recovery: emailed == Emailed::Recovery,
        };
        let sending = AtomicBool::new(false);
        self.record_signups(|signups| {
            let may = signups.may_send(&started.to, now);
            sending.store(may, Ordering::Relaxed);
            Ok(may.then(|| started.clone()))
        })
        .await?;
        // A note that no account has the address goes only where anyone may make this server
        // write to any address already, by signing up: elsewhere it would be a way to have it
        // mail strangers.
        let noted = emailed == Emailed::Recovery && !has_account && !self.takes_signups();
        if !sending.load(Ordering::Relaxed) || noted {
            return Ok(Reply::Done {});
        }
        let (subject, text) = link_mail(emailed, has_account, origin.as_str(), &code);
        let mail = Mail {
            to,
            subject: say(Language::English, &subject),
            text,
        };
        self.send(mailer, &mail).await?;
        Ok(Reply::Done {})
    }

    /// Answers `passkey/create_options` with an emailed link's `code` of the kind `emailed`:
    /// begins making the first passkey of a new account, or a new one for the account the
    /// address has, while the code opens such a link.
    pub(crate) async fn link_options(
        &self,
        code: &Secret,
        emailed: Emailed,
    ) -> Result<Reply, ApiError> {
        let gone = || gone(emailed);
        let open = match emailed {
            Emailed::Signup => self.takes_signups(),
            Emailed::Recovery => self.recovers(),
        };
        if !open {
            return Err(gone());
        }
        let accounts = self.accounts()?;
        let hash = kept(code.expose());
        let now = self.inner.harness.clock().now();
        self.catch_up_signups().await?;
        // The address its link went to: the one the code carries, which its hash covers.
        let email = code
            .expose()
            .split_once(TO)
            .and_then(|(_, to)| unhex(to))
            .filter(|_| self.signups(|signups| signups.opens(&hash, emailed, now).is_some()))
            .ok_or_else(gone)?;
        let (person, browsers) = match emailed {
            Emailed::Signup => accounts.made().await?,
            Emailed::Recovery => {
                let person = accounts.with_email(&email).await?.ok_or_else(gone)?;
                (person, accounts.browsers(person).await?)
            }
        };
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
            emailed,
        });
        Ok(crate::browsers::options(ceremony))
    }

    /// The passkey being made by an emailed link with the request `ceremony`, taken, if one is.
    pub(crate) fn signing(&self, ceremony: &str) -> Option<Signing> {
        let mut signing = self.signings();
        let at = signing.0.iter().position(|it| it.ceremony == ceremony)?;
        Some(signing.0.remove(at))
    }

    /// Answers `passkey/create` for a passkey being made by an emailed link: the link spent,
    /// then, for a sign-up, the new account's first passkey, a new organization with the person
    /// its Owner, and the browser signed in; to get back in, the account's new passkey.
    pub(crate) async fn finish_link(
        &self,
        signing: Signing,
        credential: &serde_json::Value,
        name: Option<&str>,
    ) -> Result<Reply, ApiError> {
        let accounts = self.accounts()?;
        let browsers = accounts.browsers(signing.person).await?;
        // Checked, then the link spent, and only then is anything of the account kept: a link
        // spent meanwhile leaves nothing behind.
        let checked = browsers.checked(&signing.ceremony, credential, name)?;
        self.spend(&signing).await?;
        if signing.emailed == Emailed::Recovery {
            return self.recovered(&signing, &browsers, checked).await;
        }
        // The address is this account's from now on, unless another took it meanwhile.
        if accounts.claim_email(&signing.email, signing.person).await? != signing.person {
            return Err(ApiError::new(code::NOT_ALLOWED, Sentence::SignupTaken));
        }
        let session = browsers.keep(checked).await?;
        let now = self.inner.harness.clock().now();
        let millis = u64::try_from(now.as_millisecond()).unwrap_or_default();
        let random = self.inner.harness.entropy().bytes()?;
        let org =
            OrgId::from_uuid(Builder::from_unix_timestamp_millis(millis, &random).into_uuid());
        self.found(org, signing.person, &signing.email).await?;
        accounts.join(signing.person, org).await?;
        Ok(self.signed_in(session, &browsers))
    }

    /// Spends the link that `signing` was begun with, once across servers: refused when it
    /// opens nothing any more.
    async fn spend(&self, signing: &Signing) -> Result<(), ApiError> {
        let now = self.inner.harness.clock().now();
        self.record_signups(|signups| {
            signups
                .opens(&signing.hash, signing.emailed, now)
                .ok_or_else(|| gone(signing.emailed))?;
            Ok(Some(SignupFinished {
                hash: signing.hash.clone(),
            }))
        })
        .await
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
        let now = self.inner.harness.clock().now();
        self.signups(|held| held.drop_expired(now));
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
            let chosen = self.signups(|held| make(held))?;
            let Some(payload) = chosen else {
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

/// Every emailed link's code of the kind `emailed` that opens nothing, used, expired or
/// guessed, is refused alike.
fn gone(emailed: Emailed) -> ApiError {
    let sentence = match emailed {
        Emailed::Signup => Sentence::SignupGone,
        Emailed::Recovery => Sentence::RecoveryGone,
    };
    ApiError::new(code::NO_INVITE, sentence)
}

/// The subject and text of the email a link of the kind `emailed` with `code` sends, from the
/// server at `origin`: the link, or, where the address has an account (`has_account`) for a
/// sign-up, or none to get back in, a note with none.
fn link_mail(emailed: Emailed, has_account: bool, origin: &str, code: &str) -> (Sentence, String) {
    let english = |sentence: &Sentence| say(Language::English, sentence);
    let linked = |fragment: &str, mail: &Sentence, ignore: &Sentence| {
        let link = format!("{origin}/#{fragment}={code}");
        format!("{}\n\n{link}\n\n{}\n", english(mail), english(ignore))
    };
    match (emailed, has_account) {
        (Emailed::Signup, false) => (
            Sentence::SignupMailSubject,
            linked("signup", &Sentence::SignupMail, &Sentence::SignupMailIgnore),
        ),
        (Emailed::Signup, true) => (
            Sentence::SignupHaveAccountSubject,
            format!("{}\n\n{origin}\n", english(&Sentence::SignupHaveAccount)),
        ),
        (Emailed::Recovery, true) => (
            Sentence::RecoveryMailSubject,
            linked(
                "recover",
                &Sentence::RecoveryMail,
                &Sentence::RecoveryMailIgnore,
            ),
        ),
        (Emailed::Recovery, false) => (
            Sentence::RecoveryNoAccountSubject,
            format!("{}\n\n{origin}\n", english(&Sentence::RecoveryNoAccount)),
        ),
    }
}

/// `bytes` in lowercase hex.
fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0xf])
        .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
        .collect()
}

/// The text `hex` wrote, if it is that.
fn unhex(text: &str) -> Option<String> {
    let pairs = text.as_bytes().chunks(2);
    let bytes: Option<Vec<u8>> = pairs
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}
