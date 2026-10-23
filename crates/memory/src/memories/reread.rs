//! Reading again what memory tools' results showed (`reply.rs`), for a thread being rebuilt:
//! each memory at the version shown, unless it is forgotten since, and each turn of a past
//! conversation recall found, unless recall hides it now or it no longer opens. Scoped exactly
//! as recall is, by the thread reading them now, never by what the reply names.

use std::collections::{BTreeMap, BTreeSet};

use glave_ports::memories::{MemoryContext, MemoryError, Shown};
use glave_ports::search::{Found, query_terms};
use glave_ports::{EventLog, Scope};

use super::Memories;
use super::recall::{Hidden, ledgers, snippet};
use crate::reply::{Hit, Of, Reply, Texts};

impl<L: EventLog + 'static> Memories<L> {
    /// The texts each of `replies` showed, read again for a thread of `on`, in order.
    pub(crate) async fn texts(
        &self,
        on: &MemoryContext,
        replies: &[&Reply],
    ) -> Result<Vec<Texts>, MemoryError> {
        let wanted: Vec<Shown> = replies.iter().flat_map(|reply| reply.memories()).collect();
        let memories: BTreeMap<Shown, String> = (self.versions(on, &wanted).await?)
            .into_iter()
            .map(|memory| (memory.shown(), memory.text))
            .collect();
        let mut hidden = self.hidden(on).await?;
        let ledgers = ledgers(&self.rules().readable(on.person, on.scope));
        let mut all = Vec::with_capacity(replies.len());
        for reply in replies {
            let mut texts: Texts = (reply.memories().into_iter())
                .filter_map(|shown| Some((Of::Memory(shown), memories.get(&shown)?.clone())))
                .collect();
            if let Reply::Recalled { query, hits } = reply {
                self.turns(&ledgers, &mut hidden, (query, hits), &mut texts)
                    .await;
            }
            all.push(texts);
        }
        Ok(all)
    }

    /// Into `texts`, each turn of a past conversation among `hits` as it reads now around
    /// `query`'s words, from the first of `ledgers` that holds it, unless `hidden` hides it.
    async fn turns(
        &self,
        ledgers: &[Scope],
        hidden: &mut Hidden,
        (query, hits): (&str, &[Hit]),
        texts: &mut Texts,
    ) {
        let wanted: BTreeSet<String> = query_terms(query).into_iter().collect();
        for hit in hits {
            let &Hit::Thread { thread, seq, .. } = hit else {
                continue;
            };
            let found = Found { thread, seq };
            for ledger in ledgers {
                let Some(said) = self.said_at(*ledger, found).await else {
                    continue;
                };
                // Hidden is asked only in the ledger that holds the turn: it reads the turn's
                // bounds there.
                if !hidden.hides(&self.inner.log, *ledger, found).await {
                    texts.insert(Of::Turn(thread, seq), snippet(&said.text, &wanted));
                }
                break;
            }
        }
    }
}
