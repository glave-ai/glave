//! What the browsers ledger holds, as a server knows it: the sessions and passkeys its events
//! made, with the codes waiting in memory beside them; and each change to them, as the event
//! it is recorded as.

use glave_event::{
    At, Body, BrowserSignedIn, BrowserSignedOut, BrowserUsed, EventId, Header, PasskeyAdded,
    PasskeyRemoved, PasskeyUsed, RawEvent, Seq, decode, encode,
};
use glave_ports::stream::Reserved;
use jiff::Timestamp;
use uuid::Uuid;
use webauthn_rs::prelude::Passkey;

use super::{BrowserError, SESSION_LIFETIME, new_id};

/// What is known: the codes waiting, and the sessions and passkeys as the ledger has them so
/// far.
#[derive(Default)]
pub(super) struct State {
    /// Each link's code's digest, and when it was made.
    pub(super) codes: Vec<([u8; 32], Timestamp)>,
    /// Each setup link's code's digest, and when it was made.
    pub(super) setups: Vec<([u8; 32], Timestamp)>,
    /// When each setup code that was not one was tried, lately.
    pub(super) misses: Vec<Timestamp>,
    pub(super) sessions: Vec<Kept>,
    pub(super) passkeys: Vec<KeptPasskey>,
    /// The `seq` the ledger's next event takes.
    pub(super) next: u64,
}

/// One session as the ledger has it.
pub(super) struct Kept {
    pub(super) id: Uuid,
    /// The SHA-256 of its token, in hex.
    pub(super) hash: String,
    /// When it was last recorded as used.
    pub(super) used: Timestamp,
    /// The passkey that signed it in, if one did.
    pub(super) passkey: Option<Uuid>,
}

/// One passkey as the ledger has it.
pub(super) struct KeptPasskey {
    pub(super) id: Uuid,
    pub(super) name: String,
    pub(super) added: Timestamp,
    pub(super) used: Option<Timestamp>,
    pub(super) credential: Passkey,
    /// The relying party it was made for, by its id; none for a public origin's of before.
    pub(super) party: Option<String>,
    /// Whether it was made from a link sent to the account's email.
    pub(super) emailed: bool,
}

impl KeptPasskey {
    /// Whether it signs browsers in with the relying party whose id is `party`, a public
    /// origin's when `public`: only with the one it was made for.
    pub(super) fn made_for(&self, party: &str, public: bool) -> bool {
        self.party.as_deref().map_or(public, |made| made == party)
    }
}

impl State {
    /// The sessions used within [`SESSION_LIFETIME`] of `now` whose passkey, if one signed
    /// them in, is still there.
    pub(super) fn live(&self, now: Timestamp) -> impl Iterator<Item = &Kept> {
        self.sessions.iter().filter(move |session| {
            now.duration_since(session.used) <= SESSION_LIFETIME
                && session
                    .passkey
                    .is_none_or(|passkey| self.passkeys.iter().any(|kept| kept.id == passkey))
        })
    }

    /// Takes in the ledger's event at the next `seq`; any other is already in, or not yet.
    pub(super) fn take(&mut self, raw: RawEvent) {
        if raw.seq.get() != self.next {
            return;
        }
        self.next = self.next.saturating_add(1);
        let at = raw.at.timestamp();
        let Ok(event) = decode(raw) else {
            return;
        };
        match event.body {
            Body::BrowserSignedIn(signed_in) => self.sessions.push(Kept {
                id: signed_in.session,
                hash: signed_in.hash,
                used: at,
                passkey: signed_in.passkey,
            }),
            Body::BrowserUsed(used) => {
                for session in &mut self.sessions {
                    if session.id == used.session {
                        session.used = session.used.max(at);
                    }
                }
            }
            Body::BrowserSignedOut(out) => self
                .sessions
                .retain(|session| !out.sessions.contains(&session.id)),
            Body::PasskeyAdded(added) => {
                // One whose credential cannot be read signs no one in.
                if let Ok(credential) = serde_json::from_str(&added.credential) {
                    self.passkeys.push(KeptPasskey {
                        id: added.passkey,
                        name: added.name,
                        added: at,
                        used: None,
                        credential,
                        party: added.party,
                        emailed: added.emailed,
                    });
                }
            }
            Body::PasskeyUsed(used) => self.used(&used, at),
            Body::PasskeyRemoved(removed) => {
                self.passkeys.retain(|kept| kept.id != removed.passkey);
            }
            _ => {}
        }
    }

    /// Takes in that a passkey signed a browser in at `at`.
    fn used(&mut self, used: &PasskeyUsed, at: Timestamp) {
        let Some(kept) = self
            .passkeys
            .iter_mut()
            .find(|kept| kept.id == used.passkey)
        else {
            return;
        };
        kept.used = Some(at);
        if let Some(credential) = used
            .credential
            .as_deref()
            .and_then(|credential| serde_json::from_str(credential).ok())
        {
            kept.credential = credential;
        }
    }
}

/// One change to the sessions and passkeys.
#[derive(Clone)]
pub(super) enum Change {
    /// A session began: its id, its token's hash, and the passkey it was signed in with.
    SignedIn(Uuid, String, Option<Uuid>),
    Used(Uuid),
    SignedOut(Vec<Uuid>),
    /// A passkey was made: its id, its name, its credential as JSON, and the relying party it
    /// was made for.
    PasskeyAdded(Uuid, String, String, String, bool),
    /// A passkey signed a browser in, and its credential as JSON when that changed.
    PasskeyUsed(Uuid, Option<String>),
    PasskeyRemoved(Uuid),
}

impl Change {
    /// This change as the ledger's event at `seq`, made at `now`.
    pub(super) fn event(self, seq: u64, now: Timestamp) -> Result<RawEvent, BrowserError> {
        let failed = |error: &dyn std::fmt::Display| BrowserError::Keep(error.to_string());
        let header = Header {
            id: EventId::try_from(new_id(now)?).map_err(|e| failed(&e))?,
            thread: Reserved::Browsers.id(),
            seq: Seq::new(seq).map_err(|e| failed(&e))?,
            at: At::from(now),
        };
        match self {
            Self::SignedIn(session, hash, passkey) => encode(
                header,
                &BrowserSignedIn {
                    session,
                    hash,
                    passkey,
                },
            ),
            Self::Used(session) => encode(header, &BrowserUsed { session }),
            Self::SignedOut(sessions) => encode(header, &BrowserSignedOut { sessions }),
            Self::PasskeyAdded(passkey, name, credential, party, emailed) => encode(
                header,
                &PasskeyAdded {
                    passkey,
                    name,
                    credential,
                    party: Some(party),
                    emailed,
                },
            ),
            Self::PasskeyUsed(passkey, credential) => encode(
                header,
                &PasskeyUsed {
                    passkey,
                    credential,
                },
            ),
            Self::PasskeyRemoved(passkey) => encode(header, &PasskeyRemoved { passkey }),
        }
        .map_err(|e| failed(&e))
    }
}
