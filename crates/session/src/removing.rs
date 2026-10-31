//! Taking someone out of an organization (`specs/model.md`, "Identity"): one admin action that
//! records `person.removed`, takes their glave account's membership out of the directory,
//! deletes their schedules there, and ends their watches and what they have running. Their next
//! request there is refused, and a company login lets them back only by an invite.
//!
//! Every server on the store does its own part. Whatever reads the organization's people ledger
//! and finds someone removed tells the watches here; and a server that holds a watch of someone
//! in an organization, or work of theirs, reads that ledger again every five seconds, and ends
//! what they run here, for as long as they are out. So a turn that was starting as they were
//! removed ends too.

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use glave_event::{OrgRole, PersonRemoved, Principal, PrincipalId, ScheduleDeleted, ThreadId};
use glave_harness::{Cancel, Commands};
use glave_ports::{BlobStore, Caller, EventLog, Located, ModelProvider, OrgId};
use tokio::task::JoinSet;

use crate::held::Slot;
use crate::people::{Acting, Permit, no_such_person, refuse_self};
use crate::projects::MOST_PROJECTS;
use crate::protocol::{ApiError, Reply, code};
use crate::server::Server;
use crate::words::Sentence;

/// How long a removal made on another server on the store may take to reach what this one holds
/// of the person. One read of the people ledger, per organization where a watch or work needs it.
const LOOK_AGAIN: Duration = Duration::from_secs(5);

/// Organizations whose people ledgers are read at once.
const READS: usize = 16;

/// How long one look waits for a removed person's commands to stop.
const STOPPING: Duration = Duration::from_secs(10);

/// What runs here for someone other than the person on this computer: a thread's work, which
/// `cancel` stops, or its commands in the background.
struct Theirs {
    thread: ThreadId,
    org: OrgId,
    person: PrincipalId,
    cancel: Option<Cancel>,
    commands: Arc<Commands>,
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Answers `person/remove`: `person` out of the caller's organization, when the caller may.
    /// Only an Owner removes an Owner, and never the last of an organization whose owner is no
    /// one on the server's computer. Asked again for someone removed, it finishes what a
    /// failure left undone.
    pub(crate) async fn remove_person(
        &self,
        caller: &Caller,
        person: PrincipalId,
    ) -> Result<Reply, ApiError> {
        let Acting {
            theirs,
            by,
            org,
            alone,
        } = self.acting(caller).await?;
        refuse_self(&by, person)?;
        self.refuse_local(org, person)?;
        self.record_person(org, |people| {
            // Removed already: nothing to record, and what follows is done again.
            if people.removed(person) {
                return Ok(None);
            }
            if !people.has(person) {
                return Err(no_such_person(person));
            }
            if people.role(person) == OrgRole::Owner && !theirs.contains(&Permit::Owners) {
                return Err(ApiError::new(code::NOT_ALLOWED, Sentence::OwnersOnly));
            }
            if people.last_owner(person, alone) {
                return Err(ApiError::new(code::NOT_ALLOWED, Sentence::LastOwner));
            }
            Ok(Some(PersonRemoved {
                person,
                by: by.clone(),
            }))
        })
        .await?;
        // What they run here ends now, before anything else can fail, as their watches here
        // just did.
        let theirs = self
            .running_for_people()
            .into_iter()
            .filter(|theirs| theirs.org == org && theirs.person == person)
            .collect();
        let commands = self.interrupt_removed(theirs);
        self.stop_commands(commands).await;
        if let Some(accounts) = self.inner.accounts.as_deref() {
            accounts.leave(person, org).await?;
        }
        self.delete_schedules_of(org, person, &by).await?;
        Ok(Reply::Done {})
    }

    /// Finds, for as long as it runs, the removals other servers on the store made: every five
    /// seconds. Runs until dropped.
    #[expect(
        clippy::infinite_loop,
        reason = "it runs until the server drops it, and an async fn cannot return `!`"
    )]
    pub async fn follow_removals(&self) {
        loop {
            self.look_for_removals().await;
            tokio::time::sleep(LOOK_AGAIN).await;
        }
    }

    /// Reads the people ledger of each organization where a watch or work here follows someone,
    /// which tells the watches of whoever was removed, and ends what those people run here.
    /// [`READS`] at once, so a slow organization holds up few others; each read's work ends
    /// right after it, so forgetting a ledger past the most kept loses nothing.
    pub async fn look_for_removals(&self) {
        let mut orgs: BTreeMap<OrgId, Vec<Theirs>> = BTreeMap::new();
        for theirs in self.running_for_people() {
            orgs.entry(theirs.org).or_default().push(theirs);
        }
        let watched: Vec<OrgId> = self
            .inner
            .people
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .watched()
            .collect();
        for org in watched {
            orgs.entry(org).or_default();
        }
        let mut orgs = orgs.into_iter().peekable();
        let mut commands = Vec::new();
        while orgs.peek().is_some() {
            let mut reads = JoinSet::new();
            for (org, running) in orgs.by_ref().take(READS) {
                let server = self.clone();
                reads.spawn(async move {
                    // A ledger that cannot be read now is read again next time.
                    match server.catch_up_people(org).await {
                        Ok(()) => server.interrupt_removed(running),
                        Err(error) => {
                            tracing::warn!(%error, %org, "could not read the organization's people");
                            Vec::new()
                        }
                    }
                });
            }
            commands.extend(reads.join_all().await.into_iter().flatten());
        }
        self.stop_commands(commands).await;
    }

    /// What runs here for anyone but the person on this computer, who is never removed.
    fn running_for_people(&self) -> Vec<Theirs> {
        let theirs = |thread: ThreadId, slot: &Slot| match slot {
            Slot::Busy {
                cancel,
                commands,
                at,
                person,
                ..
            } => Some(Theirs {
                thread,
                org: at.org,
                person: (*person)?,
                cancel: Some(cancel.clone()),
                commands: Arc::clone(commands),
            }),
            Slot::Idle { thread: held, .. } if held.commands().any_running() => Some(Theirs {
                thread,
                org: held.scope.located().org,
                person: held.for_person()?,
                cancel: None,
                commands: held.commands(),
            }),
            Slot::Idle { .. } => None,
        };
        self.lock()
            .iter()
            .filter_map(|(thread, slot)| theirs(*thread, slot))
            .filter(|theirs| self.refuse_local(theirs.org, theirs.person).is_ok())
            .collect()
    }

    /// Ends the turns and tasks of `running` that belong to someone removed from their
    /// organization: a turn ends interrupted, and a task as stopped by whoever removed them.
    /// Waits for nothing. Answers those of `running` of such a person with commands still
    /// running, and who removed them.
    fn interrupt_removed(&self, running: Vec<Theirs>) -> Vec<(Theirs, Principal)> {
        let mut commands = Vec::new();
        for theirs in running {
            let removed = self
                .inner
                .people
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .removed_by(theirs.org, theirs.person);
            let Some(by) = removed else {
                continue;
            };
            if let Some(cancel) = &theirs.cancel {
                self.task_stopped_by(theirs.thread, by.id);
                cancel.interrupt();
            }
            if theirs.commands.any_running() {
                commands.push((theirs, by));
            }
        }
        commands
    }

    /// Stops each thread's commands in the background, all at once and for at most
    /// [`STOPPING`]: a computer that doesn't answer holds up nothing else, and what isn't
    /// stopped by then is tried again next time.
    async fn stop_commands(&self, running: Vec<(Theirs, Principal)>) {
        let mut stops = JoinSet::new();
        for (theirs, by) in running {
            let server = self.clone();
            stops.spawn(async move {
                for command in theirs.commands.running_now().await {
                    if let Err(error) = theirs.commands.stop_for(command.id, by.clone()).await {
                        tracing::warn!(
                            error = error.message(),
                            "could not stop a removed person's command"
                        );
                    }
                }
                server.record_commands(theirs.thread).await;
            });
        }
        let _ = tokio::time::timeout(STOPPING, stops.join_all()).await;
    }

    /// Deletes every schedule of the projects `person` owns in `org`, so none fires for someone
    /// no longer in it: `by` removed them. Only they could make those.
    async fn delete_schedules_of(
        &self,
        org: OrgId,
        person: PrincipalId,
        by: &Principal,
    ) -> Result<(), ApiError> {
        let owner = Principal::person(person);
        let projects = self.inner.projects.list(org, &owner, MOST_PROJECTS).await?;
        for project in projects {
            let project = Located {
                org,
                project: project.id,
            };
            self.catch_up(project).await?;
            let schedules: Vec<_> = self
                .ledgers()
                .get(&project)
                .map(|ledger| ledger.schedules.keys().copied().collect())
                .unwrap_or_default();
            for schedule in schedules {
                let deleted = ScheduleDeleted {
                    schedule,
                    by: Some(by.clone()),
                };
                self.record_in_ledger(project, &deleted).await?;
            }
        }
        Ok(())
    }
}
