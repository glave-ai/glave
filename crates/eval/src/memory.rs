//! Memory for one attempt (`specs/memory.md`, "Evals"): a sealed store of the attempt's own, the
//! spaces its threads work in, glave's memory server over them when memory is on, the memories
//! seeded before the first thread, the past conversations `recall` searches, tidy-up when the
//! task asks for it, and afterwards what the memory ledgers hold, for the checks.

use std::collections::BTreeMap;
use std::sync::Arc;

use glave_event::{
    Body, EventId, Header, MemoryId, MemoryScope, MessageAdded, Part, Principal, PrincipalId, Role,
    Seq, ThreadId, decode, encode,
};
use glave_harness::{Clock as _, Entropy as _, OsEntropy, SystemClock, memory_ledger};
use glave_keys::{KeySettings, MemoryRoot};
use glave_memory::{GlaveMemory, Memories, Personal, Rules, SpaceMemory, Tidier, Writer};
use glave_ports::{EventLog, OrgId, Scope, SpaceId};
use glave_store_sqlite::{SqliteLog, Store};

use crate::error::EvalError;
use crate::task::Task;

/// The person every attempt works for.
pub fn person() -> Result<PrincipalId, EvalError> {
    PrincipalId::try_from(uuid::Uuid::from_u128(
        0x0192_f5c0_0000_7000_8000_0000_0000_0001,
    ))
    .map_err(|error| EvalError::Setup(error.to_string()))
}

fn org() -> OrgId {
    OrgId::from_uuid(uuid::Uuid::from_u128(1))
}

/// The space named `name`: the first thread's (`None`) is the one attempts always used.
pub fn space(name: Option<&str>) -> Scope {
    let space = match name {
        None => 2,
        Some(name) => name.bytes().fold(0x0100_u128, |sum, byte| {
            sum.wrapping_mul(31).wrapping_add(u128::from(byte))
        }),
    };
    Scope {
        org: org(),
        space: SpaceId::from_uuid(uuid::Uuid::from_u128(space)),
    }
}

/// The person's own space, where their personal memory lives.
fn personal() -> Scope {
    Scope {
        org: org(),
        space: SpaceId::from_uuid(uuid::Uuid::from_u128(100)),
    }
}

fn setting(name: Option<&str>) -> SpaceMemory {
    name.and_then(SpaceMemory::named)
        .unwrap_or(SpaceMemory::Shared)
}

/// An attempt's store, and its memory when memory is on.
pub struct Remembering {
    pub store: Store,
    pub memory: Option<Arc<GlaveMemory<SqliteLog>>>,
    /// Every space the attempt uses, the person's own first.
    scopes: Vec<Scope>,
}

/// What the memory ledgers hold once an attempt is over.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryReport {
    /// The text of every memory in force, in every space.
    pub live: Vec<String>,
    /// The text of every personal memory in force.
    pub personal: Vec<String>,
    /// Every memory forgotten.
    pub forgotten: Vec<MemoryId>,
    /// Forgotten memories whose text can still be opened.
    pub readable_forgotten: Vec<MemoryId>,
    /// The ledgers' events as stored, as JSON Lines.
    pub ledger: String,
}

impl Remembering {
    /// The store for `task`'s attempt, with memory when `on`, and its seeded memories saved.
    pub async fn open(task: &Task, on: bool) -> Result<Self, EvalError> {
        let fail = |error: &dyn std::fmt::Display| EvalError::Setup(format!("memory: {error}"));
        let root = MemoryRoot::new().map_err(|e| fail(&e))?;
        let store =
            Store::open_in_memory(KeySettings::new(Arc::new(root))).map_err(|e| fail(&e))?;
        let person = person()?;
        let mut rules = Rules {
            personal: Some(Personal {
                scope: personal(),
                person,
            }),
            ..Rules::default()
        };
        rules
            .spaces
            .insert(space(None).space, setting(task.space_memory.as_deref()));
        let mut scopes = vec![personal(), space(None)];
        for thread in &task.threads {
            let here = space(thread.space.as_deref());
            if thread.space.is_some() {
                rules
                    .spaces
                    .insert(here.space, setting(thread.space_memory.as_deref()));
            }
            if !scopes.contains(&here) {
                scopes.push(here);
            }
        }
        let memories = || {
            Memories::new(
                store.log(),
                store.keyring(),
                rules.clone(),
                Box::new(SystemClock),
                Box::new(OsEntropy),
            )
        };
        seed(&store, &rules, person, &task.memories).await?;
        for past in &task.past {
            write_past(&store, space(past.space.as_deref()), &past.messages).await?;
        }
        let memory = if on {
            Some(Arc::new(
                GlaveMemory::start(memories()).await.map_err(|e| fail(&e))?,
            ))
        } else {
            None
        };
        Ok(Self {
            store,
            memory,
            scopes,
        })
    }

    /// Tidies every space's memory now, on `model`, whether or not it is due. What each run
    /// did, for the transcript.
    pub async fn tidy(&self, model: &dyn Tidier) -> Result<Vec<String>, EvalError> {
        let Some(memory) = &self.memory else {
            return Ok(Vec::new());
        };
        let mut done = Vec::new();
        for scope in &self.scopes {
            let tidied = memory
                .memories()
                .tidy(*scope, model)
                .await
                .map_err(|e| EvalError::Setup(format!("tidy-up failed: {e}")))?;
            done.push(format!(
                "merged {}, forgot {}, asked {}, refused {:?}",
                tidied.merged, tidied.forgotten, tidied.questions, tidied.refused
            ));
        }
        Ok(done)
    }

    /// What the memory ledgers hold now.
    pub async fn report(&self) -> Result<MemoryReport, EvalError> {
        let fail = |error: &dyn std::fmt::Display| EvalError::Setup(format!("memory: {error}"));
        let ledger = memory_ledger().ok_or_else(|| fail(&"no ledger id"))?;
        let mut report = MemoryReport::default();
        for scope in &self.scopes {
            let raw = self
                .store
                .log()
                .read(scope, ledger, None, usize::MAX)
                .await
                .map_err(|e| fail(&e))?;
            let mut texts: BTreeMap<MemoryId, (MemoryScope, String)> = BTreeMap::new();
            let mut forgotten = Vec::new();
            for event in raw {
                report
                    .ledger
                    .push_str(&event.to_json().map_err(|e| fail(&e))?);
                report.ledger.push('\n');
                // A memory whose key was destroyed reads as erased: it does not decode.
                match decode(event).map(|event| event.body) {
                    Ok(Body::MemorySaved(saved)) => {
                        texts.insert(saved.memory, (saved.scope, saved.text));
                    }
                    Ok(Body::MemoryEdited(edited)) => {
                        if let Some(entry) = texts.get_mut(&edited.memory) {
                            entry.1 = edited.text;
                        }
                    }
                    Ok(Body::MemoryForgotten(gone)) => forgotten.push(gone.memory),
                    Ok(Body::MemoryRestored(restored)) => {
                        forgotten.retain(|memory| *memory != restored.memory);
                    }
                    _ => {}
                }
            }
            for memory in &forgotten {
                if texts.remove(memory).is_some() {
                    report.readable_forgotten.push(*memory);
                }
            }
            for (scope, text) in texts.into_values() {
                if scope == MemoryScope::Personal {
                    report.personal.push(text.clone());
                }
                report.live.push(text);
            }
            report.forgotten.extend(forgotten);
        }
        if let Some(memory) = &self.memory {
            memory.close().await;
        }
        Ok(report)
    }
}

/// Saves `seeds` before the first thread, each as if that many days ago.
async fn seed(
    store: &Store,
    rules: &Rules,
    person: PrincipalId,
    seeds: &[crate::task::Seeded],
) -> Result<(), EvalError> {
    let fail = |error: &dyn std::fmt::Display| EvalError::Setup(format!("memory: {error}"));
    let now = SystemClock.now();
    for (order, seed) in (0_i64..).zip(seeds) {
        // Saved that many days ago (a second apart, in the order listed), so which came
        // later is as plain as in life.
        let saved = now
            .checked_sub(jiff::SignedDuration::from_hours(
                i64::from(seed.days_ago).saturating_mul(24),
            ))
            .and_then(|day| day.checked_add(jiff::SignedDuration::from_secs(order)))
            .map_err(|e| fail(&e))?;
        let seeding = Memories::new(
            store.log(),
            store.keyring(),
            rules.clone(),
            Box::new(Fixed(saved)),
            Box::new(OsEntropy),
        );
        let writer = Writer {
            person: Some(person),
            scope: space(None),
            by: Principal::person(person),
            source: None,
            untrusted_read: seed.untrusted,
        };
        let which = MemoryScope::named(&seed.scope).unwrap_or(MemoryScope::Space);
        seeding
            .remember(&writer, &seed.text, which, None, true)
            .await
            .map_err(|e| fail(&e))?;
    }
    Ok(())
}

/// A clock that always reads one time: when a seeded memory was saved.
struct Fixed(jiff::Timestamp);

impl glave_harness::Clock for Fixed {
    fn now(&self) -> jiff::Timestamp {
        self.0
    }
}

/// A past conversation in `scope`: the person's `messages`, in a thread of their own.
async fn write_past(store: &Store, scope: Scope, messages: &[String]) -> Result<(), EvalError> {
    let fail = |error: &dyn std::fmt::Display| EvalError::Setup(format!("past thread: {error}"));
    let now = SystemClock.now();
    let thread = ThreadId::new(now, OsEntropy.bytes()).map_err(|e| fail(&e))?;
    let mut events = Vec::new();
    for (seq, text) in (1_u64..).zip(messages) {
        let header = Header {
            id: EventId::new(now, OsEntropy.bytes()).map_err(|e| fail(&e))?,
            thread,
            seq: Seq::new(seq).map_err(|e| fail(&e))?,
            at: now.into(),
        };
        let message = MessageAdded {
            role: Role::User,
            parts: vec![Part::Text { text: text.clone() }],
            author: None,
        };
        events.push(encode(header, &message).map_err(|e| fail(&e))?);
    }
    store
        .log()
        .append(&scope, &events)
        .await
        .map_err(|e| fail(&e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaces_are_named_apart_and_the_first_is_the_usual_one() {
        assert_eq!(
            space(None).space,
            SpaceId::from_uuid(uuid::Uuid::from_u128(2))
        );
        assert_ne!(space(Some("client")), space(Some("other")));
        assert_eq!(space(Some("client")), space(Some("client")));
        assert_ne!(space(Some("client")), personal());
    }
}
