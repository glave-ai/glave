//! Tidy-up (`specs/memory.md`, "Tidy-up"): one ledger's memories kept small and true. A cheap
//! model (the gateway's `summary` route) proposes merges of duplicates and contradictions; every
//! proposal is checked here before anything changes, so tidy-up only merges, edits and forgets
//! memories that exist, never invents a fact, never loses what the person asked to keep, and
//! never touches a memory saved after untrusted content. A contradiction goes to the more
//! recent memory; when it is unclear, both stay and the person is asked. Every change is an
//! event `by` glave, and glave's forgets wait 30 days before their keys are destroyed, so each
//! can be undone until then.

mod merge;
mod plan;

use std::collections::{BTreeMap, BTreeSet};

use glave_event::{At, MemoryEdited, MemoryId, MemoryTidied, Principal, PrincipalId};
use glave_ports::memories::MemoryError;
use glave_ports::{EventLog, ModelProvider, Payer, Scope};
use jiff::SignedDuration;

use self::merge::keeps_facts;
use self::plan::{Proposal, ask};
use super::Memories;
use super::write::{BUDGET_TOKENS, Erasure, checked, tokens};
use crate::book::Entry;
use crate::similar::words;

/// How long glave's own forgets wait before their keys are destroyed, so they can be undone.
const UNDO_DAYS: i64 = 30;
/// Tidy-up runs at least this often over a ledger that has something to tidy.
const EVERY_DAYS: i64 = 7;
/// Above this share of the budget, in percent, tidy-up runs (and compacts) sooner.
const NEAR_PERCENT: usize = 90;
/// Over the budget's edge, tidy-up runs again at most this often, in hours.
const NEAR_EVERY_HOURS: i64 = 1;

/// The cheap model tidy-up asks: the gateway's `summary` route.
pub struct Tidier<P> {
    pub provider: P,
    /// The model named in the request; a gateway routes by the request's purpose.
    pub model: String,
}

/// What one tidy-up did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tidied {
    /// Memories that took in others: each an edit by glave.
    pub merged: usize,
    /// Memories forgotten into a merge or for a more recent one, undoable for 30 days.
    pub forgotten: usize,
    /// Contradictions left for the person to settle.
    pub questions: usize,
    /// Proposals refused, and why: they changed nothing.
    pub refused: Vec<String>,
}

fn days_from(at: At, days: i64) -> At {
    At::from(
        at.timestamp()
            .checked_add(SignedDuration::from_hours(days.saturating_mul(24)))
            .unwrap_or(at.timestamp()),
    )
}

impl<L: EventLog + 'static> Memories<L> {
    /// Whether `ledger` is due a tidy-up: it has memories tidy-up may change, and it has not
    /// run for a week, or the ledger passed 90% of its budget and it has not run for an hour.
    pub async fn tidy_due(&self, ledger: Scope) -> Result<bool, MemoryError> {
        let now = self.now();
        self.with_book(ledger, |book| {
            let live = book.live();
            let tidy_able = live.iter().filter(|memory| !memory.untrusted_read).count();
            if tidy_able < 2 {
                return false;
            }
            let used: usize = live.iter().map(|memory| tokens(&memory.text)).sum();
            let near = used.saturating_mul(100) > BUDGET_TOKENS.saturating_mul(NEAR_PERCENT);
            book.tidied.is_none_or(|last| {
                let since = now.timestamp().duration_since(last.timestamp());
                since >= SignedDuration::from_hours(EVERY_DAYS.saturating_mul(24))
                    || (near && since >= SignedDuration::from_hours(NEAR_EVERY_HOURS))
            })
        })
        .await
    }

    /// Tidies `ledger` now, asking `model`, as the work of `person`, who reads it, when anyone
    /// does. Records that it ran, with any contradiction it leaves for the person, even when
    /// nothing changed.
    pub async fn tidy<P: ModelProvider>(
        &self,
        ledger: Scope,
        person: Option<PrincipalId>,
        model: &Tidier<P>,
    ) -> Result<Tidied, MemoryError> {
        let (entries, open) = self
            .with_book(ledger, |book| {
                let entries: Vec<Entry> = book
                    .entries
                    .values()
                    .filter(|entry| entry.forgot.is_none())
                    .cloned()
                    .collect();
                (entries, book.open_questions())
            })
            .await?;
        let used: usize = entries
            .iter()
            .filter_map(Entry::live)
            .map(|memory| tokens(&memory.text))
            .sum();
        let near = used.saturating_mul(100) > BUDGET_TOKENS.saturating_mul(NEAR_PERCENT);
        // Memories saved after untrusted content are never shown to the model nor changed, and
        // neither are those waiting on the person's answer.
        let waiting: BTreeSet<MemoryId> = open.iter().flatten().copied().collect();
        let candidates: Vec<Entry> = entries
            .into_iter()
            .filter(|entry| !entry.untrusted_read && !waiting.contains(&entry.memory))
            .collect();
        let mut tidied = Tidied::default();
        let mut questions = Vec::new();
        if candidates.len() >= 2 {
            // An answer that cannot be read changes nothing; the run is still recorded. The
            // work of the person whose memory it is: their filters choose who serves it.
            let payer = Payer {
                org: ledger.org(),
                person,
            };
            let proposals = match ask(model, &candidates, near, payer).await? {
                Ok(proposals) => proposals,
                Err(why) => {
                    tidied.refused.push(why);
                    Vec::new()
                }
            };
            let mut used_ids = BTreeSet::new();
            for proposal in proposals {
                let outcome = self
                    .settle(ledger, &candidates, &proposal, &mut used_ids)
                    .await?;
                match outcome {
                    Settled::Merged(forgotten) => {
                        tidied.merged = tidied.merged.saturating_add(1);
                        tidied.forgotten = tidied.forgotten.saturating_add(forgotten);
                    }
                    Settled::Superseded => tidied.forgotten = tidied.forgotten.saturating_add(1),
                    Settled::Asked(pair) => questions.push(pair),
                    Settled::Refused(why) => tidied.refused.push(why),
                }
            }
        }
        tidied.questions = questions.len();
        let record = MemoryTidied {
            questions,
            by: Principal::glave(),
        };
        self.append(ledger, &record).await?;
        Ok(tidied)
    }

    /// Checks one proposal and carries it out.
    async fn settle(
        &self,
        ledger: Scope,
        candidates: &[Entry],
        proposal: &Proposal,
        used: &mut BTreeSet<MemoryId>,
    ) -> Result<Settled, MemoryError> {
        let picked = match pick(candidates, proposal.memories(), used) {
            Ok(picked) => picked,
            Err(why) => return Ok(Settled::Refused(why)),
        };
        let glave = Principal::glave();
        let erase_at = days_from(self.now(), UNDO_DAYS);
        match proposal {
            Proposal::Merge { text, .. } => {
                if picked.len() < 2 {
                    return Ok(Settled::Refused(
                        "a merge needs two memories or more".into(),
                    ));
                }
                let text = match checked(text) {
                    Ok(text) => text,
                    Err(error) => return Ok(Settled::Refused(error.to_string())),
                };
                let sources: Vec<String> = picked.iter().map(|(_, text)| text.clone()).collect();
                if let Err(why) = keeps_facts(&text, &sources) {
                    return Ok(Settled::Refused(why));
                }
                // The one stated last takes the merged text first, so the merge dates from the
                // newest statement that went into it, and stopping halfway leaves a duplicate,
                // never a loss.
                let mut said: Vec<(Option<At>, MemoryId)> = (picked.iter())
                    .map(|(entry, _)| (entry.stated(), entry.memory))
                    .collect();
                said.sort();
                let ids: Vec<MemoryId> = said.into_iter().map(|(_, memory)| memory).collect();
                let Some((keep, rest)) = ids.split_last() else {
                    return Ok(Settled::Refused(
                        "a merge needs two memories or more".into(),
                    ));
                };
                let edited = MemoryEdited {
                    memory: *keep,
                    text,
                    by: glave.clone(),
                };
                self.append(ledger, &edited).await?;
                for memory in rest {
                    self.forget_in(ledger, *memory, &glave, Erasure::Merged(erase_at, *keep))
                        .await?;
                }
                used.extend(ids.iter().copied());
                Ok(Settled::Merged(rest.len()))
            }
            Proposal::Conflict { clear, .. } => {
                let [(one, one_text), (other, other_text)] = picked.as_slice() else {
                    return Ok(Settled::Refused(
                        "a contradiction is between two memories".into(),
                    ));
                };
                // Two memories about one fact share a word about it; two that share none are a
                // misread list, and forgetting either would lose a fact.
                if words(one_text).is_disjoint(&words(other_text)) {
                    return Ok(Settled::Refused(format!(
                        "memories {:?} share no word, so neither replaces the other",
                        proposal.memories()
                    )));
                }
                let (one_at, other_at) = (one.stated(), other.stated());
                used.extend([one.memory, other.memory]);
                if !clear || one_at == other_at {
                    return Ok(Settled::Asked([one.memory, other.memory]));
                }
                let older = if one_at < other_at { one } else { other };
                self.forget_in(ledger, older.memory, &glave, Erasure::At(erase_at))
                    .await?;
                Ok(Settled::Superseded)
            }
        }
    }
}

/// What became of one proposal.
enum Settled {
    /// Merged, forgetting this many into the one kept.
    Merged(usize),
    /// The older of a contradiction was forgotten.
    Superseded,
    /// An unclear contradiction, left for the person.
    Asked([MemoryId; 2]),
    Refused(String),
}

/// The candidates a proposal names, by their number in the list the model was shown, each
/// with its current text: each at most once, none already used.
fn pick<'a>(
    candidates: &'a [Entry],
    numbers: &[usize],
    used: &BTreeSet<MemoryId>,
) -> Result<Vec<(&'a Entry, String)>, String> {
    let mut picked: BTreeMap<MemoryId, (&Entry, String)> = BTreeMap::new();
    for number in numbers {
        let entry = number
            .checked_sub(1)
            .and_then(|index| candidates.get(index))
            .ok_or_else(|| format!("there is no memory {number} in the list"))?;
        if picked.contains_key(&entry.memory) {
            return Err(format!("memory {number} is named twice"));
        }
        if used.contains(&entry.memory) {
            return Err(format!("memory {number} was already tidied in this run"));
        }
        let text = entry
            .current()
            .map(|(_, version)| version.text.clone())
            .ok_or_else(|| format!("memory {number} has no text"))?;
        picked.insert(entry.memory, (entry, text));
    }
    Ok(picked.into_values().collect())
}
