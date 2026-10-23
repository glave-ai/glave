//! Memory for one attempt (`specs/memory.md`, "Evals"): a sealed store of the attempt's own, the
//! projects its threads work in, glave's memory server over them when memory is on, the memories
//! seeded before the first thread, the past conversations `recall` searches, tidy-up when the
//! task asks for it, and afterwards what the memory ledgers hold, for the checks, and what memory
//! held and cost, for the result.

mod ledger;

use std::collections::BTreeMap;
use std::sync::Arc;

use glave_event::{
    EventId, Header, MemoryId, MemoryScope, MessageAdded, Part, Principal, PrincipalId, Role, Seq,
    ThreadId, encode,
};
use glave_keys::{InMemoryRoot, KeySettings};
use glave_memory::{GlaveMemory, Memories, Personal, Rules, Tidier, Writer, tokens};
use glave_ports::memories::ProjectMemory;
use glave_ports::stream::Reserved;
use glave_ports::{
    Clock as _, Entropy as _, EventLog, ModelProvider, OrgId, OsEntropy, ProjectId, Scope,
    SystemClock,
};
use glave_store_sqlite::{SqliteLog, Store};
use serde::{Deserialize, Serialize};

use self::ledger::Ledger;
use crate::calendar::Calendar;
use crate::error::EvalError;
use crate::task::Task;

/// What a thread names as its project to work in the person's Chats.
const CHATS: &str = "chats";

/// The person every attempt works for.
pub(crate) fn person() -> Result<PrincipalId, EvalError> {
    PrincipalId::try_from(uuid::Uuid::from_u128(
        0x0192_f5c0_0000_7000_8000_0000_0000_0001,
    ))
    .map_err(|error| EvalError::Setup(error.to_string()))
}

fn org() -> OrgId {
    OrgId::from_uuid(uuid::Uuid::from_u128(1))
}

/// The project named `name`: the first thread's (`None`) is the one attempts always used, and
/// `chats` is the person's own.
pub(crate) fn project(name: Option<&str>) -> Scope {
    let project = match name {
        None => 2,
        Some(CHATS) => return personal(),
        Some(name) => name.bytes().fold(0x0100_u128, |sum, byte| {
            sum.wrapping_mul(31).wrapping_add(u128::from(byte))
        }),
    };
    Scope::glaves_own(org(), ProjectId::from_uuid(uuid::Uuid::from_u128(project)))
}

/// The person's own project, their Chats, where their personal memory lives.
fn personal() -> Scope {
    Scope::glaves_own(org(), ProjectId::from_uuid(uuid::Uuid::from_u128(100)))
}

fn setting(name: Option<&str>) -> ProjectMemory {
    name.and_then(ProjectMemory::named)
        .unwrap_or(ProjectMemory::Shared)
}

/// An attempt's store, and its memory when memory is on.
pub(crate) struct Remembering {
    pub store: Store,
    pub memory: Option<Arc<GlaveMemory<SqliteLog>>>,
    person: PrincipalId,
    /// Every project the attempt uses, the person's own first, each with what the result calls
    /// it: `personal`, `project` (the first thread's), or its name.
    scopes: Vec<(String, Scope)>,
}

/// What the memory ledgers hold once an attempt is over.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryReport {
    /// The text of every memory in force, in every project.
    pub live: Vec<String>,
    /// The text of every personal memory in force.
    pub personal: Vec<String>,
    /// Every memory forgotten.
    pub forgotten: Vec<MemoryId>,
    /// Forgotten memories whose text can still be opened.
    pub readable_forgotten: Vec<MemoryId>,
    /// The ledgers' events as stored, as JSON Lines.
    pub ledger: String,
    /// What memory held and cost, when it was on.
    pub usage: Option<MemoryUse>,
}

/// What memory held and cost over an attempt, for its result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryUse {
    /// Tidy-up's runs, over every ledger.
    pub tidy_runs: u32,
    /// What tidy-up forgot: memories merged into another, or replaced by a later one.
    pub tidied_away: Vec<String>,
    /// About how many tokens the memories in force take at the end, as the budget counts them
    /// (about 1,000 each): `personal`, `chats` (project memories saved in the person's Chats),
    /// `project` (the first thread's) and each other project by name.
    pub used: BTreeMap<String, usize>,
    /// About how many tokens of memory each thread started with, as the budget counts them, in
    /// the order they ran.
    pub in_view: Vec<usize>,
}

impl Remembering {
    /// The store for `task`'s attempt, with memory when `on`, read and written on `calendar`'s
    /// days, and its seeded memories saved.
    pub(crate) async fn open(
        task: &Task,
        on: bool,
        calendar: &Calendar,
    ) -> Result<Self, EvalError> {
        let fail = |error: &dyn std::fmt::Display| EvalError::Setup(format!("memory: {error}"));
        let root = InMemoryRoot::new().map_err(|e| fail(&e))?;
        let store =
            Store::open_in_memory(KeySettings::new(Arc::new(root))).map_err(|e| fail(&e))?;
        let person = person()?;
        let mut rules = Rules {
            personal: Some(Personal {
                scope: personal(),
                person,
                on: true,
            }),
            ..Rules::default()
        };
        rules.projects.insert(
            project(None).project(),
            setting(task.project_memory.as_deref()),
        );
        let mut scopes = vec![
            ("personal".to_owned(), personal()),
            ("project".to_owned(), project(None)),
        ];
        for thread in &task.threads {
            let here = project(thread.project.as_deref());
            if thread.project.is_some() {
                rules
                    .projects
                    .insert(here.project(), setting(thread.project_memory.as_deref()));
            }
            if !scopes.iter().any(|(_, scope)| *scope == here) {
                scopes.push((thread.project.clone().unwrap_or_default(), here));
            }
        }
        seed(&store, &rules, person, &task.memories).await?;
        for past in &task.past {
            write_past(&store, project(past.project.as_deref()), &past.messages).await?;
        }
        let memory = if on {
            let memories = Memories::new(
                store.log(),
                store.keyring(),
                rules,
                Box::new(calendar.clone()),
                Box::new(OsEntropy),
            );
            Some(Arc::new(
                GlaveMemory::start(memories).await.map_err(|e| fail(&e))?,
            ))
        } else {
            None
        };
        Ok(Self {
            store,
            memory,
            person,
            scopes,
        })
    }

    /// Tidies every project's memory now, on `model`, whether or not it is due. What each run
    /// did, for the transcript.
    pub(crate) async fn tidy<P: ModelProvider>(
        &self,
        model: &Tidier<P>,
    ) -> Result<Vec<String>, EvalError> {
        let Some(memory) = &self.memory else {
            return Ok(Vec::new());
        };
        let mut done = Vec::new();
        for (name, scope) in &self.scopes {
            let tidied = memory
                .memories()
                .tidy(*scope, model)
                .await
                .map_err(|e| EvalError::Setup(format!("tidy-up failed: {e}")))?;
            done.push(format!(
                "{name}: merged {}, forgot {}, asked {}, refused {:?}",
                tidied.merged, tidied.forgotten, tidied.questions, tidied.refused
            ));
        }
        Ok(done)
    }

    /// Tidies on `model` in the background from now on, as the server does: each ledger a save
    /// or an edit changes, and each one `tidy_soon` asks for, once it is due.
    pub(crate) fn tidy_as_served<P: ModelProvider + 'static>(&self, model: Tidier<P>) {
        if let Some(memory) = &self.memory {
            memory.tidy_with(model);
        }
    }

    /// Asks tidy-up in the background to look at the ledgers a thread in `scope` reads, as a
    /// session does when it starts.
    pub(crate) fn tidy_soon(&self, scope: Scope) {
        if let Some(memory) = &self.memory {
            memory.tidy_soon(&self.ledgers(scope));
        }
    }

    /// The ledgers a thread in `scope` reads: the person's, in a shared project, and its own.
    fn ledgers(&self, scope: Scope) -> Vec<Scope> {
        let Some(memory) = &self.memory else {
            return Vec::new();
        };
        let mut ledgers: Vec<Scope> = (memory.memories().rules())
            .readable(Some(self.person), scope)
            .into_iter()
            .map(|(_, ledger)| ledger)
            .collect();
        // The person's Chats hold both their personal memory and the project's.
        ledgers.dedup();
        ledgers
    }

    /// About how many tokens of memory a thread in `scope` would start with now, as the budget
    /// counts them; `None` when memory is off.
    pub(crate) async fn in_view(&self, scope: Scope) -> Result<Option<usize>, EvalError> {
        let Some(memory) = &self.memory else {
            return Ok(None);
        };
        let shown = memory
            .memories()
            .list_as(Some(self.person), scope, None)
            .await
            .map_err(|e| EvalError::Setup(format!("memory: {e}")))?;
        Ok(Some(shown.iter().map(|memory| tokens(&memory.text)).sum()))
    }

    /// What the memory ledgers hold now, once memory has stopped: a tidy-up still running in
    /// the background is stopped first, so the ledgers read the same throughout.
    pub(crate) async fn report(&self) -> Result<MemoryReport, EvalError> {
        if let Some(memory) = &self.memory {
            memory.close().await;
        }
        let fail = |error: &dyn std::fmt::Display| EvalError::Setup(format!("memory: {error}"));
        let mut report = MemoryReport::default();
        let mut usage = MemoryUse::default();
        for (name, scope) in &self.scopes {
            let raw = self
                .store
                .log()
                .read(scope, Reserved::Memory.id(), None, usize::MAX)
                .await
                .map_err(|e| fail(&e))?;
            for event in &raw {
                report
                    .ledger
                    .push_str(&event.to_json().map_err(|e| fail(&e))?);
                report.ledger.push('\n');
            }
            let read = Ledger::read(raw);
            usage.tidy_runs = usage.tidy_runs.saturating_add(read.tidied);
            report.readable_forgotten.extend(read.readable_forgotten());
            usage.tidied_away.extend(read.by_glave());
            for (kind, text) in read.live() {
                let place = match kind {
                    MemoryScope::Personal => "personal",
                    MemoryScope::Project if *scope == personal() => CHATS,
                    MemoryScope::Project => name.as_str(),
                };
                let used = usage.used.entry(place.to_owned()).or_default();
                *used = used.saturating_add(tokens(text));
                if kind == MemoryScope::Personal {
                    report.personal.push(text.clone());
                }
                report.live.push(text.clone());
            }
            report.forgotten.extend(read.forgotten);
        }
        report.usage = self.memory.is_some().then_some(usage);
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
            scope: project(None),
            by: Principal::person(person),
            source: None,
            untrusted_read: seed.untrusted,
        };
        let which = MemoryScope::named(&seed.scope).unwrap_or(MemoryScope::Project);
        seeding
            .remember(&writer, &seed.text, which, None, true)
            .await
            .map_err(|e| fail(&e))?;
    }
    Ok(())
}

/// A clock that always reads one time: when a seeded memory was saved.
struct Fixed(jiff::Timestamp);

impl glave_ports::Clock for Fixed {
    fn now(&self) -> jiff::Timestamp {
        self.0
    }
}

/// A past conversation in `scope`: the person's `messages`, in a thread of their own.
async fn write_past(store: &Store, scope: Scope, messages: &[String]) -> Result<(), EvalError> {
    let fail = |error: &dyn std::fmt::Display| EvalError::Setup(format!("past thread: {error}"));
    let now = SystemClock.now();
    let thread =
        ThreadId::new(now, OsEntropy.bytes().map_err(|e| fail(&e))?).map_err(|e| fail(&e))?;
    let mut events = Vec::new();
    for (seq, text) in (1_u64..).zip(messages) {
        let header = Header {
            id: EventId::new(now, OsEntropy.bytes().map_err(|e| fail(&e))?)
                .map_err(|e| fail(&e))?,
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
    fn projects_are_named_apart_and_the_first_is_the_usual_one() {
        assert_eq!(
            project(None).project(),
            ProjectId::from_uuid(uuid::Uuid::from_u128(2))
        );
        assert_ne!(project(Some("client")), project(Some("other")));
        assert_eq!(project(Some("client")), project(Some("client")));
        assert_ne!(project(Some("client")), personal());
        assert_eq!(project(Some("chats")), personal(), "the person's Chats");
    }

    #[test]
    fn a_thread_asks_tidy_up_about_the_ledgers_it_reads() {
        let dir = glave_ports::testing::TempDir::new("eval-ledgers").unwrap();
        std::fs::write(dir.join("instruction.md"), "Hi.").unwrap();
        let threads = "[[glave.threads]]\nproject = \"chats\"\nmessages = [\"a\"]\n\
                       [[glave.threads]]\nproject = \"secret\"\nproject_memory = \"isolated\"\n\
                       messages = [\"b\"]\n";
        std::fs::write(dir.join("task.toml"), threads).unwrap();
        let task = crate::task::load(&dir).unwrap();
        let clock = glave_ports::testing::ManualClock::at_second(1_800_000_000);
        let calendar = Calendar::new(Arc::new(clock));
        glave_ports::block_on!(async {
            let on = Remembering::open(&task, true, &calendar).await.unwrap();
            let (first, secret) = (project(None), project(Some("secret")));
            assert_eq!(on.ledgers(first), [personal(), first]);
            assert_eq!(on.ledgers(personal()), [personal()], "Chats hold both");
            assert_eq!(on.ledgers(secret), [secret], "isolated");
            on.report().await.unwrap();
            let off = Remembering::open(&task, false, &calendar).await.unwrap();
            assert_eq!(off.ledgers(first), []);
        });
    }
}
