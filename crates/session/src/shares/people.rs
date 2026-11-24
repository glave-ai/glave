//! Specific people (`specs/session.md`, "Sharing"): a share for a list of addresses, each person
//! proving theirs with a code glave emails there. The page asks for an address and sends a code
//! only to one the share lists, answering alike either way, the mail going after the answer; the
//! right code, good once and for ten minutes, gives the browser a proof for that share's name
//! alone, good for thirty days, and checked on every request against the share as it stands.
//! Codes and proofs are kept in the shares ledger as their SHA-256 alone (`share.code.sent`,
//! `share.opened`), and so are the addresses they were for.

use std::net::IpAddr;

use glave_event::{At, Audience, Body, ShareCodeSent, ShareOpened, ShareSet, decode};
use glave_identity::signin::{kept, one_time_code, same_kept};
use glave_ports::stream::{Reserved, catch_up};
use glave_ports::{BlobStore, Caller, EventLog, Mail, ModelProvider, Scope};
use jiff::SignedDuration;

use super::SHARES;
use crate::mailing::{just_now, read_address};
use crate::protocol::{ApiError, code};
use crate::server::Server;
use crate::words::{Language, Sentence, say};

/// The most addresses one share lists.
pub(crate) const MOST_PEOPLE: usize = 50;

/// How long a code works.
const CODE_FOR: SignedDuration = SignedDuration::from_mins(10);

/// How long a proof works on the browser that holds it.
pub(crate) const PROOF_FOR: SignedDuration = SignedDuration::from_hours(30 * 24);

/// The letters a code is written in, none two alike to the eye.
const CODE_LETTERS: &[u8; 32] = b"abcdefghjkmnpqrstuvwxyz234567890";

/// How many letters a code has: 40 bits.
const CODE_LENGTH: usize = 8;

/// `people` as a share lists them: each address read as glave reads every address, once, at
/// most [`MOST_PEOPLE`]; refused for anything but addresses, or for none.
pub(crate) fn listed(people: &[String]) -> Result<Vec<String>, ApiError> {
    let mut read = Vec::with_capacity(people.len());
    for each in people {
        let address = read_address(each)
            .ok_or_else(|| ApiError::new(code::INVALID_PARAMS, Sentence::NotAnEmail))?;
        if !read.contains(&address) {
            read.push(address);
        }
    }
    if read.is_empty() || read.len() > MOST_PEOPLE {
        return Err(ApiError::new(
            code::INVALID_PARAMS,
            Sentence::SharePeopleCount {
                most: MOST_PEOPLE as u64,
            },
        ));
    }
    Ok(read)
}

/// A new code: [`CODE_LENGTH`] letters from the operating system's randomness.
fn new_code() -> Result<String, ApiError> {
    let mut random = [0u8; CODE_LENGTH];
    getrandom::fill(&mut random).map_err(|error| ApiError::internal(&error))?;
    Ok(random
        .iter()
        .map(|byte| {
            char::from(
                CODE_LETTERS
                    .get(usize::from(byte % 32))
                    .copied()
                    .unwrap_or(b'a'),
            )
        })
        .collect())
}

/// `typed` as a code is compared: lowercase, without spaces or dashes.
fn as_typed(typed: &str) -> String {
    typed
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

/// What a share's ledger says of its codes and proofs.
#[derive(Default)]
struct Proofs {
    /// Each code sent, with when, by `seq`.
    sent: Vec<(u64, At, ShareCodeSent)>,
    /// Each proof given, by `seq`.
    opened: Vec<(u64, ShareOpened)>,
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The codes sent and proofs given for `share` in `scope`.
    async fn proofs(&self, scope: &Scope, share: &ShareSet) -> Result<Proofs, ApiError> {
        let mut proofs = Proofs::default();
        let log = self.inner.harness.log();
        catch_up(log, scope, Reserved::Shares.id(), None, |page| {
            for raw in page {
                let (seq, at) = (raw.seq.get(), raw.at);
                match decode(raw).map(|event| event.body) {
                    Ok(Body::ShareCodeSent(sent)) if sent.share == share.share => {
                        proofs.sent.push((seq, at, sent));
                    }
                    Ok(Body::ShareOpened(opened)) if opened.share == share.share => {
                        proofs.opened.push((seq, opened));
                    }
                    _ => {}
                }
            }
        })
        .await?;
        Ok(proofs)
    }

    /// Sends a code to `to` when `share` lists it, at most one a minute to an address and
    /// counted against `from`'s network, the mail going after the answer; answered alike
    /// whether or not it lists it.
    pub(crate) async fn send_code(
        &self,
        scope: &Scope,
        share: &ShareSet,
        to: &str,
        from: IpAddr,
        language: Language,
    ) -> Result<(), ApiError> {
        let (Some(mailer), Audience::People { people }) = (&self.inner.mail, &share.audience)
        else {
            return Ok(());
        };
        let Some(address) = read_address(to).filter(|address| people.contains(address)) else {
            self.send_later(std::sync::Arc::clone(mailer), None);
            return Ok(());
        };
        let to_kept = kept(&address);
        let now = self.inner.harness.clock().now();
        let proofs = self.proofs(scope, share).await?;
        let recent = (proofs.sent.iter().rev())
            .find(|(_, _, sent)| sent.to == to_kept)
            .is_some_and(|(_, at, _)| just_now(at.timestamp(), now));
        if recent {
            self.send_later(std::sync::Arc::clone(mailer), None);
            return Ok(());
        }
        self.count_asked(&Caller::no_one(from)).await?;
        let code = new_code()?;
        let sent = ShareCodeSent {
            share: share.share,
            to: to_kept,
            code: kept(&code),
        };
        self.keep_latest(scope, Reserved::Shares.id(), &SHARES, &sent)
            .await?;
        let shown = code.to_uppercase();
        let (half, rest) = shown.split_at(CODE_LENGTH / 2);
        let mail = Mail {
            to: address,
            subject: say(language, &Sentence::ShareCodeSubject),
            text: say(
                language,
                &Sentence::ShareCodeText {
                    code: format!("{half}-{rest}"),
                },
            ),
        };
        self.send_later(std::sync::Arc::clone(mailer), Some(mail));
        Ok(())
    }

    /// A proof for `to` on `share`, when `typed` is the last code sent there, unspent and within
    /// [`CODE_FOR`]: the browser's to keep, its SHA-256 kept here. None for any other.
    pub(crate) async fn open_with_code(
        &self,
        scope: &Scope,
        share: &ShareSet,
        to: &str,
        typed: &str,
    ) -> Result<Option<String>, ApiError> {
        let Some(address) = read_address(to) else {
            return Ok(None);
        };
        let to_kept = kept(&address);
        let now = self.inner.harness.clock().now();
        let proofs = self.proofs(scope, share).await?;
        let last = (proofs.sent.iter().rev()).find(|(_, _, sent)| sent.to == to_kept);
        let Some((seq, at, sent)) = last else {
            return Ok(None);
        };
        let used =
            (proofs.opened.iter()).any(|(after, opened)| after > seq && opened.to == to_kept);
        let fresh = at
            .timestamp()
            .checked_add(CODE_FOR)
            .is_ok_and(|by| by > now);
        if used || !fresh || !same_kept(&kept(&as_typed(typed)), &sent.code) {
            return Ok(None);
        }
        let proof = one_time_code().map_err(|error| ApiError::internal(&error))?;
        let until = now
            .checked_add(PROOF_FOR)
            .map_err(|error| ApiError::internal(&error))?;
        let opened = ShareOpened {
            share: share.share,
            to: to_kept,
            proof: kept(proof.expose()),
            until: At::from(until),
        };
        self.keep_latest(scope, Reserved::Shares.id(), &SHARES, &opened)
            .await?;
        Ok(Some(proof.expose().to_owned()))
    }

    /// Whether `proof` lets its browser open `share` now: given for it, within its time, for an
    /// address the share still lists.
    pub(crate) async fn proven(
        &self,
        scope: &Scope,
        share: &ShareSet,
        proof: &str,
    ) -> Result<bool, ApiError> {
        let Audience::People { people } = &share.audience else {
            return Ok(true);
        };
        let held = kept(proof);
        let now = self.inner.harness.clock().now();
        let proofs = self.proofs(scope, share).await?;
        Ok(proofs.opened.iter().any(|(_, opened)| {
            same_kept(&opened.proof, &held)
                && opened.until.timestamp() > now
                && people.iter().any(|address| kept(address) == opened.to)
        }))
    }
}
