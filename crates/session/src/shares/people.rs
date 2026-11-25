//! Specific people (`specs/session.md`, "Sharing"): a share for a list of addresses, each person
//! proving theirs by opening a one-time link glave emails there. The page asks for an address;
//! the answer is the same, and as quick, whether or not the share lists it, since the network's
//! count comes first and everything else happens after, in a task the server owns: whether the
//! share lists the address, the link's record and the mail. A link is good once, for ten
//! minutes, for its share, its address and the list of people as it was when sent; asking again
//! leaves earlier links working, so no one asking in someone's name locks them out. Opening one
//! gives the browser a proof for that share's name alone, good for thirty days while the list
//! of people stays as it was. Tokens and proofs are kept in the shares ledger as their SHA-256
//! alone, with the address's (`share.link.sent`, `share.opened`); each server keeps what it read
//! of each project's ledger in a bounded cache, reading on from where it last read.

use std::collections::{BTreeMap, HashMap};
use std::net::IpAddr;
use std::sync::PoisonError;

use glave_event::{
    At, Audience, Body, RawEvent, Seq, ShareId, ShareLinkSent, ShareOpened, ShareSet,
};
use glave_identity::signin::{kept, one_time_code, same_kept};
use glave_ports::stream::{Reserved, append_retrying, catch_up, in_order};
use glave_ports::{BlobStore, Caller, EventLog, Located, Mail, ModelProvider, Scope};
use jiff::SignedDuration;

use super::SHARES;
use crate::mailing::{just_now, read_address};
use crate::protocol::{ApiError, code};
use crate::server::Server;
use crate::words::{Language, Sentence, say};

/// The most addresses one share lists.
pub(crate) const MOST_PEOPLE: usize = 50;

/// How long an emailed link works.
const LINK_FOR: SignedDuration = SignedDuration::from_mins(10);

/// How long a proof works on the browser that holds it.
pub(crate) const PROOF_FOR: SignedDuration = SignedDuration::from_hours(30 * 24);

/// The most links sent for one share within [`LINK_FOR`], to every address together, so asking
/// for many addresses at once sends few.
const MOST_SENT: usize = 10;

/// The most projects' ledgers a server keeps read in memory; past them, it starts over, and
/// reads each again from its start when next asked.
const MOST_LEDGERS: usize = 256;

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

/// A project's shares ledger as far as this server has read it: its live shares, each with the
/// `seq` its list of people was last set at, and, for each, the links sent and proofs given
/// since then. Nothing is trimmed but what can open nothing any more: everything of a share
/// stopped, or given for a list of people it no longer has, and links past [`LINK_FOR`] and
/// proofs past their `until` ([`Ledger::prune`]). So a link is never spent twice, and no
/// recipient whose proof still works is asked again; what is kept is bounded by what can still
/// open a live share.
/// A live share as its ledger last set it.
#[derive(Debug, Clone)]
pub(crate) struct Live {
    pub(crate) set: ShareSet,
    /// When it was last set.
    pub(crate) at: At,
    /// The `seq` its list of people was last set at.
    given: u64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Ledger {
    next: u64,
    pub(crate) live: BTreeMap<ShareId, Live>,
    sent: BTreeMap<ShareId, Vec<(u64, At, ShareLinkSent)>>,
    opened: BTreeMap<ShareId, Vec<ShareOpened>>,
}

impl Ledger {
    fn take(&mut self, raw: RawEvent) {
        let seq = raw.seq.get();
        let Some((at, body)) = in_order(&mut self.next, raw) else {
            return;
        };
        match body {
            Body::ShareSet(set) => {
                let given = match self.live.get(&set.share) {
                    Some(before) if before.set.audience == set.audience => before.given,
                    _ => {
                        // A new list of people: what was given for the old one opens nothing.
                        self.sent.remove(&set.share);
                        self.opened.remove(&set.share);
                        seq
                    }
                };
                self.live.insert(set.share, Live { set, at, given });
            }
            Body::ShareStopped(stopped) => {
                self.live.remove(&stopped.share);
                self.sent.remove(&stopped.share);
                self.opened.remove(&stopped.share);
            }
            Body::ShareLinkSent(sent) => {
                self.sent
                    .entry(sent.share)
                    .or_default()
                    .push((seq, at, sent));
            }
            Body::ShareOpened(opened) => {
                self.opened.entry(opened.share).or_default().push(opened);
            }
            _ => {}
        }
    }

    /// The `seq` `share`'s list of people was last set at, while it is live.
    fn given(&self, share: ShareId) -> Option<u64> {
        self.live.get(&share).map(|live| live.given)
    }
}

impl Ledger {
    /// Lets go of what can open nothing at `now`: links past [`LINK_FOR`], used or not, and
    /// proofs past their `until`. A link kept is never spent twice, as its proof names it.
    fn prune(&mut self, now: jiff::Timestamp) {
        self.sent.retain(|_, sent| {
            sent.retain(|(_, at, _)| {
                at.timestamp()
                    .checked_add(LINK_FOR)
                    .is_ok_and(|by| by > now)
            });
            !sent.is_empty()
        });
        self.opened.retain(|_, opened| {
            opened.retain(|opened| opened.until.timestamp() > now);
            !opened.is_empty()
        });
    }

    fn sent(&self, share: ShareId) -> &[(u64, At, ShareLinkSent)] {
        self.sent.get(&share).map_or(&[], Vec::as_slice)
    }

    fn opened(&self, share: ShareId) -> &[ShareOpened] {
        self.opened.get(&share).map_or(&[], Vec::as_slice)
    }
}

/// What each server has read of projects' shares ledgers, by project. Who may read it is
/// settled before it is: the owner's requests are let through for their project first, and a
/// link's page reads only the project the shares' list names for that link; the cache itself
/// checks nothing.
pub(crate) type Ledgers = HashMap<Located, Ledger>;

/// Whether another link may go to the address whose SHA-256 is `to`, after the links `sent`
/// for its share: none to it within the minute, and fewer than [`MOST_SENT`] to anyone within
/// [`LINK_FOR`].
fn room(sent: &[(u64, At, ShareLinkSent)], to: &str, now: jiff::Timestamp) -> bool {
    let within = |at: &At| {
        at.timestamp()
            .checked_add(LINK_FOR)
            .is_ok_and(|by| by > now)
    };
    let recent = (sent.iter().rev())
        .find(|(_, _, sent)| sent.to == to)
        .is_some_and(|(_, at, _)| just_now(at.timestamp(), now));
    !recent && sent.iter().filter(|(_, at, _)| within(at)).count() < MOST_SENT
}

/// Whether `share` lists the address whose SHA-256 is `to`.
fn lists(share: &ShareSet, to: &str) -> bool {
    match &share.audience {
        Audience::People { people } => people.iter().any(|address| kept(address) == to),
        Audience::Anyone => false,
    }
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// `scope`'s shares ledger, read on from where this server last read it.
    pub(crate) async fn share_ledger(&self, scope: &Scope) -> Result<Ledger, ApiError> {
        let at = scope.located();
        let held = |ledgers: &mut Ledgers| ledgers.get(&at).cloned().unwrap_or_default();
        let mut ledger = self.held_ledgers(held);
        let after = Seq::new(ledger.next.saturating_sub(1)).ok();
        let log = self.inner.harness.log();
        catch_up(log, scope, Reserved::Shares.id(), after, |page| {
            for raw in page {
                ledger.take(raw);
            }
        })
        .await?;
        ledger.prune(self.inner.harness.clock().now());
        self.held_ledgers(|ledgers| {
            if ledgers.len() >= MOST_LEDGERS && !ledgers.contains_key(&at) {
                ledgers.clear();
            }
            ledgers.insert(at, ledger.clone());
        });
        Ok(ledger)
    }

    fn held_ledgers<R>(&self, look: impl FnOnce(&mut Ledgers) -> R) -> R {
        let mut ledgers = (self.inner.share_ledgers)
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        look(&mut ledgers)
    }

    /// Asks for a link to `share` to be emailed to `to`: counted against `from`'s network, then
    /// answered at once, alike whether or not the share lists it; the rest happens after, in a
    /// task the server owns, the link written in `language`.
    pub(crate) async fn ask_link(
        &self,
        scope: Scope,
        share: ShareSet,
        to: String,
        (from, language): (IpAddr, Language),
    ) -> Result<(), ApiError> {
        self.count_asked(&Caller::no_one(from)).await?;
        let server = self.clone();
        self.own(tokio::spawn(async move {
            if let Err(error) = server.send_link(&scope, &share, &to, language).await {
                tracing::warn!(error = %error.message, "a share's link was not sent");
            }
        }));
        Ok(())
    }

    /// Emails a link to `share` to `to` when the share lists it and the mail server takes it,
    /// at most one a minute to an address.
    async fn send_link(
        &self,
        scope: &Scope,
        share: &ShareSet,
        to: &str,
        language: Language,
    ) -> Result<(), ApiError> {
        let Some(mailer) = &self.inner.mail else {
            return Ok(());
        };
        let address = read_address(to).filter(|address| mailer.reaches(address));
        let Some(address) = address else {
            return Ok(());
        };
        let to_kept = kept(&address);
        let ledger = self.share_ledger(scope).await?;
        let now = self.inner.harness.clock().now();
        let (Some(live), Some(given)) = (ledger.live.get(&share.share), ledger.given(share.share))
        else {
            return Ok(());
        };
        if !lists(&live.set, &to_kept) || !room(ledger.sent(share.share), &to_kept, now) {
            return Ok(());
        }
        let token = one_time_code().map_err(|error| ApiError::internal(&error))?;
        let sent = ShareLinkSent {
            share: share.share,
            to: to_kept,
            token: kept(token.expose()),
            given,
        };
        self.keep_latest(scope, Reserved::Shares.id(), &SHARES, &sent)
            .await?;
        let port = self.reached_port();
        let page = (self.inner.shares.as_ref())
            .map(|domain| domain.link(&share.link, port))
            .unwrap_or_default();
        let mail = Mail {
            to: address,
            subject: say(language, &Sentence::ShareLinkSubject),
            text: format!(
                "{}\n\n{page}open/{}\n\n{}\n",
                say(language, &Sentence::ShareLinkText),
                token.expose(),
                say(language, &Sentence::ShareLinkNote),
            ),
        };
        self.send(mailer.as_ref(), &mail).await
    }

    /// A proof for the browser that opened the link whose token is `token`, when it is one sent
    /// for `share`, unused, within [`LINK_FOR`], for an address the share still lists and its
    /// list of people as it is: the browser's to keep, its SHA-256 kept here. None for any other.
    pub(crate) async fn open_with_link(
        &self,
        scope: &Scope,
        share: &ShareSet,
        token: &str,
    ) -> Result<Option<String>, ApiError> {
        let held = kept(token);
        let proof = one_time_code().map_err(|error| ApiError::internal(&error))?;
        let log = self.inner.harness.log();
        let ledger_id = Reserved::Shares.id();
        // Checked on the ledger as read, and written right after it, at its next place: a link
        // opened meanwhile by anyone, on any server, takes that place first, so this one reads
        // the ledger again and finds the link used.
        let opened = append_retrying(log, scope, ledger_id, || async {
            let ledger = self.share_ledger(scope).await?;
            let Some(opened) = self.opening(&ledger, share, &held, proof.expose())? else {
                return Ok(None);
            };
            let seq = Seq::new(ledger.next.max(1)).map_err(|error| ApiError::internal(&error))?;
            let event = self.ledger_event(ledger_id, seq, &opened)?;
            Ok::<_, ApiError>(Some((vec![event], ())))
        });
        Ok(opened.await?.map(|_| proof.expose().to_owned()))
    }

    /// What opening `share` with the link whose token's SHA-256 is `held` records, giving
    /// `proof`: when it is a link sent for `share`, unused, within [`LINK_FOR`], for an address
    /// the share still lists and its list of people as it is. None for any other.
    fn opening(
        &self,
        ledger: &Ledger,
        share: &ShareSet,
        held: &str,
        proof: &str,
    ) -> Result<Option<ShareOpened>, ApiError> {
        let now = self.inner.harness.clock().now();
        let found =
            (ledger.sent(share.share).iter()).find(|(_, _, sent)| same_kept(&sent.token, held));
        let Some((seq, at, sent)) = found else {
            return Ok(None);
        };
        let used = (ledger.opened(share.share).iter()).any(|opened| opened.link == *seq);
        let fresh = at
            .timestamp()
            .checked_add(LINK_FOR)
            .is_ok_and(|by| by > now);
        let current = ledger.given(share.share) == Some(sent.given);
        if used || !fresh || !current || !lists(share, &sent.to) {
            return Ok(None);
        }
        let until = now
            .checked_add(PROOF_FOR)
            .map_err(|error| ApiError::internal(&error))?;
        Ok(Some(ShareOpened {
            share: share.share,
            to: sent.to.clone(),
            proof: kept(proof),
            link: *seq,
            given: sent.given,
            until: At::from(until),
        }))
    }

    /// Whether `proof` lets its browser open `share` now: given for it, within its time, for an
    /// address the share still lists and its list of people as it is now.
    pub(crate) fn proven(&self, ledger: &Ledger, share: &ShareSet, proof: &str) -> bool {
        if !matches!(share.audience, Audience::People { .. }) {
            return true;
        }
        let held = kept(proof);
        let now = self.inner.harness.clock().now();
        let given = ledger.given(share.share);
        ledger.opened(share.share).iter().any(|opened| {
            same_kept(&opened.proof, &held)
                && opened.until.timestamp() > now
                && Some(opened.given) == given
                && lists(share, &opened.to)
        })
    }

    /// The port this server is reached at on this computer, for a link at `localhost`.
    pub(crate) fn reached_port(&self) -> Option<u16> {
        self.inner.reached.get().and_then(|reached| {
            let at = reached.at.as_deref()?;
            at.rsplit_once(':')?.1.parse().ok()
        })
    }
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;

    use super::*;

    fn sent_to(to: &str, at: Timestamp) -> (u64, At, ShareLinkSent) {
        let share = ShareId::new(Timestamp::UNIX_EPOCH, [1; 10]).unwrap();
        let sent = ShareLinkSent {
            share,
            to: to.to_owned(),
            token: String::new(),
            given: 1,
        };
        (1, At::from(at), sent)
    }

    #[test]
    fn a_share_sends_one_link_a_minute_to_an_address_and_a_few_in_ten_minutes_to_all() {
        let start = Timestamp::from_second(1_800_000_000).unwrap();
        let later = |secs: i64| start.checked_add(SignedDuration::from_secs(secs)).unwrap();
        let mut sent = vec![sent_to("ada", start)];
        assert!(!room(&sent, "ada", later(30)), "not twice in a minute");
        assert!(room(&sent, "ada", later(61)));
        sent.extend((1..MOST_SENT).map(|n| sent_to(&format!("other-{n}"), later(1))));
        assert!(
            !room(&sent, "bea", later(120)),
            "{MOST_SENT} in ten minutes"
        );
        assert!(room(&sent, "bea", later(601)), "ten minutes on, again");
    }
}
