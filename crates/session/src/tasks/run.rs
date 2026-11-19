//! A schedule's run as a task, claimed by its own `task.started`: recorded before its turn
//! starts, and only while its project's ledger shows nothing of that run yet, so of servers
//! that both fire it only the one whose claim lands starts it. A run whose turn then can't
//! start is ended at once; one whose server stops between the two is left open, and recorded
//! interrupted by the server that recovers tasks next, so the person learns it was lost.

use glave_event::{Fired, Outcome, TaskEnded, TaskStarted, ThreadId, TokenCounts};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, title_from};

use super::Work;
use crate::leased::elsewhere;
use crate::ledger::Ledger;
use crate::protocol::{ApiError, TaskInfo};
use crate::server::Server;

/// Why a schedule's run did not start, and whether this server had claimed it first.
pub(crate) struct Unstarted {
    pub(crate) refused: ApiError,
    pub(crate) claimed: bool,
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Starts `work`, the schedule's run `fired`, in its chat `thread` for `caller`, with
    /// `budget`, once its claim lands: none when another server's landed first.
    pub(crate) async fn start_run(
        &self,
        caller: &Caller,
        thread: ThreadId,
        work: Work,
        budget: u64,
        fired: Fired,
    ) -> Result<Option<TaskInfo>, Unstarted> {
        let before = |refused| Unstarted {
            refused,
            claimed: false,
        };
        let project = (self.may_start_task(caller, thread, true).await)
            .map_err(before)?
            .located();
        // What refuses it before its claim is recorded once for the same reason, as a skip is;
        // its project's folder gone is the one met most.
        (self.folder_found(project).await).map_err(before)?;
        let Work(prompt, choice) = &work;
        let model = (self.model_of(thread, choice.model.as_deref()).await).map_err(before)?;
        let started = TaskStarted {
            thread,
            title: title_from(prompt).unwrap_or_default(),
            model,
            budget: Some(budget),
            schedule: Some(fired),
            by: self.someone_else(caller.person),
        };
        // Its task's lease first, so a server recovering tasks never takes its claim, made
        // before its turn starts, for one left by a server that stopped.
        let lease = (self.take_task(project.org, thread).await).map_err(before)?;
        let lease = lease.ok_or_else(|| before(elsewhere(thread)))?;
        let there = |ledger: &Ledger| ledger.settled(fired.schedule, fired.due);
        let claimed = self.record_unless(project, &started, there).await;
        if !matches!(claimed, Ok(true)) {
            self.let_go(&lease).await;
            return claimed.map(|_| None).map_err(before);
        }
        let claim = Some((started, lease));
        let run = self.run_task(caller, (project, thread), work, Some(budget), claim);
        match run.await {
            Ok(info) => Ok(Some(info)),
            Err(refused) => {
                // Claimed, it ends here, so no later run waits on it as still going.
                let ended = TaskEnded {
                    thread,
                    due: Some(fired.due),
                    outcome: Outcome::Failed,
                    tokens: TokenCounts::default(),
                    cost: None,
                    by: None,
                };
                if let Err(error) = self.record_in_ledger(project, &ended).await {
                    tracing::warn!(%error, %thread, "a run that did not start was not ended");
                }
                Err(Unstarted {
                    refused,
                    claimed: true,
                })
            }
        }
    }
}
