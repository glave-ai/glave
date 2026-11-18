//! A schedule's run waiting for the person when its server stops, adopted by the next server
//! that runs tasks: once, under its task's lease, however many servers look, and its end
//! recorded once, for that run; let go of again when that server stops too; recorded
//! interrupted when it can't be adopted, the others still recovered; and what its subagents
//! spent before counted against its budget.

use std::path::Path;
use std::sync::Arc;

use glave_event::{Body, Outcome, TaskEnded, ThreadId, decode};
use glave_ports::stream::Reserved;
use glave_ports::testing::ManualClock;
use glave_ports::{EventLog, InMemoryLeases, Leases};
use glave_session::protocol::{Answer, Request};
use glave_session::{Answering, Repaired};
use serde_json::json;

use crate::schedule_runs::{asking, at, open_tasks, schedule, set, writes_after_asking};
use crate::support::*;

/// Another server on `first`'s store and `leases`, numbered `seed`, answering `answers`.
async fn another(
    first: &Tested<Shared>,
    (log, leases, now): (&Shared, &Arc<InMemoryLeases>, &ManualClock),
    repo: &Path,
    seed: u64,
    answers: Vec<glave_ports::Completion>,
) -> Tested<Shared> {
    let mut setup = Setup::asking(answers, Answering::Clients);
    setup.clock = now.clone();
    setup.seed = seed;
    setup.projects = Some(Arc::clone(first.projects()));
    setup.settings.person = first.person();
    setup.leases = Some(Arc::clone(leases) as Arc<dyn Leases>);
    setup.worktrees = repo.parent().map(|dir| dir.join("worktrees"));
    let (server, _) = setup.build_on(repo, log.clone()).await;
    server
}

/// Every `task.ended` the ledger holds for `thread`.
async fn ends(log: &Shared, thread: ThreadId) -> Vec<TaskEnded> {
    let at = scope();
    let ledger = log.read(&at, Reserved::Tasks.id(), None, usize::MAX);
    (ledger.await.unwrap().into_iter())
        .filter_map(|raw| match decode(raw).unwrap().body {
            Body::TaskEnded(ended) if ended.thread == thread => Some(ended),
            _ => None,
        })
        .collect()
}

fn done() -> Vec<glave_ports::Completion> {
    vec![answer(Some("Done."), None)]
}

/// Makes `first` a schedule with `budget` whose run, due at 11:00, waits for the person, then
/// stops it; answers the run's chat and what it waits at.
async fn waiting_when_it_stops(
    first: &Tested<Shared>,
    now: &ManualClock,
    budget: u64,
) -> (ThreadId, glave_event::CallId) {
    let chat = schedule(first, "Careful", "0 11 * * *", budget).await;
    let chat = chat.thread.unwrap();
    set(now, "2026-10-10T11:00:00Z");
    assert_eq!(first.tick().await.unwrap(), vec![chat]);
    let approval = asked_in(first, chat).await;
    first.shutdown().await;
    (chat, approval)
}

#[test]
fn a_waiting_run_is_adopted_once_however_many_servers_look() {
    block_on!(async {
        let log = Shared::default();
        let (first, now, repo, _dir) =
            asking("adopted-once", writes_after_asking(), log.clone()).await;
        let (chat, approval) = waiting_when_it_stops(&first, &now, 10_000).await;
        let (_, leases) = shared_leases();
        let store = (&log, &leases, &now);
        let second = another(&first, store, &repo, 2, done()).await;
        let third = another(&first, store, &repo, 3, done()).await;
        assert_eq!(second.recover_tasks().await.unwrap(), Repaired::default());
        let held = Repaired { closed: 0, held: 1 };
        assert_eq!(
            third.recover_tasks().await.unwrap(),
            held,
            "the second follows it"
        );
        let decided = Request::ApprovalDecide {
            thread: chat,
            approval,
            decision: Answer::AllowOnce,
        };
        assert!(second.request(decided).await.is_ok());
        settle(&second).await;
        let ended = ends(&log, chat).await;
        assert_eq!(ended.len(), 1, "{ended:?}");
        assert_eq!(ended[0].due, Some(at("2026-10-10T11:00:00Z")), "that run's");
        assert_eq!(third.recover_tasks().await.unwrap(), Repaired::default());
        second.shutdown().await;
        third.shutdown().await;
    });
}

#[test]
fn a_server_stops_while_an_adopted_run_waits_and_the_next_adopts_it_again() {
    block_on!(async {
        let log = Shared::default();
        let (first, now, repo, _dir) =
            asking("adopted-again", writes_after_asking(), log.clone()).await;
        let (chat, _) = waiting_when_it_stops(&first, &now, 10_000).await;
        let (_, leases) = shared_leases();
        let store = (&log, &leases, &now);
        let second = another(&first, store, &repo, 2, done()).await;
        assert_eq!(second.recover_tasks().await.unwrap(), Repaired::default());
        // It follows the run by what its thread records, and lets go of it as it stops.
        second.shutdown().await;
        let third = another(&first, store, &repo, 3, done()).await;
        assert_eq!(third.recover_tasks().await.unwrap(), Repaired::default());
        assert_eq!(open_tasks(&third).await, 1, "followed here now");
        assert_eq!(ends(&log, chat).await, Vec::new(), "never interrupted");
        third.shutdown().await;
    });
}

#[test]
fn runs_that_can_t_be_adopted_are_recorded_interrupted_and_recovery_goes_on() {
    block_on!(async {
        let write = json!({"path": "note.md", "content": "hi"});
        let asks = || costing(answer(None, Some(("write", write.clone()))), 10, 5, 1);
        let log = Shared::default();
        let (first, now, repo, _dir) =
            asking("adopted-not", vec![asks(), asks()], log.clone()).await;
        let one = schedule(&first, "One", "0 11 * * *", 10_000).await;
        let two = schedule(&first, "Two", "0 11 * * *", 10_000).await;
        set(&now, "2026-10-10T11:00:00Z");
        assert_eq!(first.tick().await.unwrap().len(), 2);
        asked_in(&first, one.thread.unwrap()).await;
        asked_in(&first, two.thread.unwrap()).await;
        first.shutdown().await;
        let (_, leases) = shared_leases();
        let second = another(&first, (&log, &leases, &now), &repo, 2, done()).await;
        // What they spent can't be read, so neither can be adopted.
        log.fail_on("model.responded");
        let recovered = second.recover_tasks().await.unwrap();
        assert_eq!(recovered, Repaired { closed: 2, held: 0 });
        second.shutdown().await;
    });
}

#[test]
fn an_adopted_run_counts_what_its_subagents_spent_against_its_budget() {
    block_on!(async {
        let brief = json!({"description": "look around", "prompt": "Look around."});
        let write = json!({"path": "note.md", "content": "hi"});
        let answers = vec![
            costing(answer(None, Some(("agent", brief))), 10, 5, 1),
            costing(answer(Some("Looked."), None), 10, 5, 1),
            costing(answer(None, Some(("write", write))), 10, 5, 1),
        ];
        let log = Shared::default();
        let mut setup = Setup::asking(answers, Answering::Clients);
        setup.settings.subagents = true;
        setup.clock =
            ManualClock::at_millis(at("2026-10-10T10:03:00Z").timestamp().as_millisecond());
        let now = setup.clock.clone();
        let (first, _, repo, _dir) = setup
            .build_in_repository("adopted-subagents", log.clone())
            .await;
        // 50 tokens: 45 spent before it stops, the subagent's 15 among them; the next call's
        // go past, and without the subagent's would not.
        let (chat, approval) = waiting_when_it_stops(&first, &now, 50).await;
        let (_, leases) = shared_leases();
        let last = vec![costing(answer(Some("Done."), None), 10, 5, 1)];
        let second = another(&first, (&log, &leases, &now), &repo, 2, last).await;
        assert_eq!(second.recover_tasks().await.unwrap(), Repaired::default());
        let decided = Request::ApprovalDecide {
            thread: chat,
            approval,
            decision: Answer::AllowOnce,
        };
        assert!(second.request(decided).await.is_ok());
        settle(&second).await;
        let ran = tasks(&second).await;
        let run = ran.iter().find(|task| task.thread == chat).unwrap();
        assert_eq!(run.outcome, Some(Outcome::LimitReached), "{run:?}");
        second.shutdown().await;
    });
}
