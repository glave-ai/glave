//! Firing every run once: two servers on one store that both believe they fire start a run
//! once, the one whose claim (its `task.started`) lands, and a claim whose server stopped
//! before the turn began is recorded lost, never run.

use std::sync::Arc;

use glave_event::{
    At, Body, EventId, Fired, Header, Outcome, Payload, Principal, ScheduleId, Seq, TaskStarted,
    ThreadId, When, decode, encode,
};
use glave_ports::stream::Reserved;
use glave_ports::testing::{FaultyLog, ManualClock, TempDir};
use glave_ports::{Caller, EventLog as _, InMemoryProjects, ProjectId, Projects, Scope};
use glave_session::protocol::{Reply, Request, ScheduleInfo};
use jiff::Timestamp;

use crate::support::*;

pub(crate) fn set(now: &ManualClock, text: &str) {
    now.set_millis(text.parse::<Timestamp>().unwrap().as_millisecond());
}

/// A server with plenty of short answers, its clock at 10:03 until the test moves it.
pub(crate) fn setup() -> Setup {
    let mut setup = Setup::new((0..10).map(|_| answer(Some("Done."), None)).collect());
    setup.clock = ManualClock::at_millis(
        "2026-10-10T10:03:00Z"
            .parse::<Timestamp>()
            .unwrap()
            .as_millisecond(),
    );
    setup
}

/// A schedule `caller` makes in their Chats, every day at 11:00.
pub(crate) async fn daily<L: glave_ports::EventLog + 'static>(
    server: &Tested<L>,
    caller: &Caller,
) -> Result<ScheduleInfo, i64> {
    let request = Request::ScheduleCreate {
        project: None,
        name: None,
        when: When::Cron("0 11 * * *".into()),
        zone: None,
        prompt: "check the build".into(),
        budget: 5_000,
    };
    match server.request_as(caller, request).await {
        Ok(Reply::Schedule { schedule }) => Ok(schedule),
        Ok(other) => panic!("a schedule, not {other:?}"),
        Err(error) => Err(error.code),
    }
}

/// What `project`'s task ledger records.
pub(crate) async fn ledger_of(log: &Shared, project: ProjectId) -> Vec<Body> {
    let ledger = Scope::glaves_own(scope().org(), project);
    let events = log
        .read(&ledger, Reserved::Tasks.id(), None, usize::MAX)
        .await
        .unwrap();
    (events.into_iter())
        .map(|raw| decode(raw).unwrap().body)
        .collect()
}

/// Who each task started in `project`'s ledger ran as, and for which schedule.
pub(crate) async fn runs_in(
    log: &Shared,
    project: ProjectId,
) -> Vec<(Option<Principal>, ScheduleId)> {
    (ledger_of(log, project).await.into_iter())
        .filter_map(|body| match body {
            Body::TaskStarted(started) => {
                Some((started.by, started.schedule.map(|fired| fired.schedule)?))
            }
            _ => None,
        })
        .collect()
}

/// A server in the fresh folder `name` on one store, `log` and projects it keeps for the next
/// server: the server, the projects, its clock, at 10:03, its repository and the folder.
pub(crate) async fn on_a_store(
    name: &str,
    log: &Shared,
) -> (
    Tested<Shared>,
    Arc<dyn Projects>,
    ManualClock,
    std::path::PathBuf,
    TempDir,
) {
    let projects: Arc<dyn Projects> = Arc::new(InMemoryProjects::new());
    let mut first = setup();
    first.projects = Some(Arc::clone(&projects));
    let now = first.clock.clone();
    let (server, _, repo, dir) = first.build_in_repository(name, log.clone()).await;
    (server, projects, now, repo, dir)
}

/// Appends `payload` at `seq` to `thread` in `scope`, at 10:00 that morning, as another server
/// would.
pub(crate) async fn append<T: Payload>(
    log: &Shared,
    scope: &Scope,
    thread: ThreadId,
    seq: u64,
    payload: &T,
) {
    let at = At::try_from("2026-10-10T10:00:00.000Z".to_owned()).unwrap();
    let mut random = [7; 10];
    random[..8].copy_from_slice(&seq.to_be_bytes());
    let header = Header {
        id: EventId::new(at.timestamp(), random).unwrap(),
        thread,
        seq: Seq::new(seq).unwrap(),
        at,
    };
    log.append(scope, &[encode(header, payload).unwrap()])
        .await
        .unwrap();
}

#[test]
fn two_servers_that_both_fire_start_a_run_once() {
    block_on!(async {
        let log = Shared::default();
        let faulty = FaultyLog::over(Arc::clone(&log.0));
        // One store: its log, and its projects; each server keeps leases of its own, so both
        // believe they fire the store's schedules.
        let projects: Arc<dyn Projects> = Arc::new(InMemoryProjects::new());
        let mut first = setup();
        first.projects = Some(Arc::clone(&projects));
        let now = first.clock.clone();
        let (one, _, repo, _dir) = first
            .build_in_repository("firing-twice", faulty.clone())
            .await;
        let made = daily(&one, &Caller::default()).await.unwrap();
        let mut second = setup();
        second.clock = now.clone();
        second.projects = Some(projects);
        second.seed = 2;
        let (two, _) = second.build_on(&repo, log.clone()).await;
        set(&now, "2026-10-10T11:00:00Z");
        // The first's claim, its run's start, is held as it is written, while the second ticks
        // through.
        faulty.hold("task.started");
        let racing = async {
            faulty.held().await;
            let fired = two.tick().await;
            faulty.release();
            fired
        };
        let (from_one, from_two) = tokio::join!(one.tick(), racing);
        let mut started = from_one.unwrap();
        started.extend(from_two.unwrap());
        assert_eq!(started, vec![made.thread.unwrap()], "started once");
        settle(&one).await;
        settle(&two).await;
        assert_eq!(
            runs_in(&log, made.project).await.len(),
            1,
            "one claim landed; the other read it and wrote none"
        );
    });
}

/// A daily schedule at 11:00 in the server's own project, which a server started since
/// recovers too.
async fn own_daily(server: &Tested<Shared>) -> ScheduleInfo {
    let request = Request::ScheduleCreate {
        project: Some(scope().project()),
        name: None,
        when: When::Cron("0 11 * * *".into()),
        zone: None,
        prompt: "check the build".into(),
        budget: 5_000,
    };
    let Ok(Reply::Schedule { schedule }) = server.request(request).await else {
        panic!("a schedule");
    };
    schedule
}

/// Records `made`'s 11:00 run claimed, as a server that stopped before its turn began left it.
async fn claim_left(log: &Shared, made: &ScheduleInfo) {
    let fired = Fired {
        schedule: made.schedule,
        due: At::try_from("2026-10-10T11:00:00.000Z".to_owned()).unwrap(),
    };
    let claim = TaskStarted {
        thread: made.thread.unwrap(),
        title: "check the build".into(),
        model: "test-model".into(),
        budget: Some(5_000),
        schedule: Some(fired),
        by: None,
    };
    let ledger = Scope::glaves_own(scope().org(), made.project);
    let next = u64::try_from(ledger_of(log, made.project).await.len()).unwrap() + 1;
    append(log, &ledger, Reserved::Tasks.id(), next, &claim).await;
}

/// Whether `made`'s ledger records a task of it interrupted.
async fn interrupted(log: &Shared, made: &ScheduleInfo) -> bool {
    (ledger_of(log, made.project).await.into_iter())
        .any(|body| matches!(body, Body::TaskEnded(ended) if ended.outcome == Outcome::Interrupted))
}

#[test]
fn a_run_claimed_by_a_server_that_stopped_before_starting_it_is_recorded_lost() {
    block_on!(async {
        let log = Shared::default();
        let (server, projects, now, repo, _dir) = on_a_store("firing-lost", &log).await;
        let made = own_daily(&server).await;
        let person = server.person();
        server.shutdown().await;
        drop(server);
        claim_left(&log, &made).await;
        set(&now, "2026-10-10T11:00:30Z");
        let mut later = setup();
        later.clock = now.clone();
        later.projects = Some(projects);
        later.worktrees = repo.parent().map(|dir| dir.join("worktrees"));
        // The same person's server, started again.
        later.settings.person = person;
        let (restarted, _) = later.build_on(&repo, log.clone()).await;
        let recovered = restarted.recover_tasks().await.unwrap();
        assert_eq!(
            recovered,
            glave_session::Repaired { closed: 1, held: 0 },
            "recorded interrupted: the person learns it was lost"
        );
        assert_eq!(
            restarted.tick().await.unwrap(),
            Vec::new(),
            "never run twice"
        );
        assert!(interrupted(&log, &made).await);
    });
}

#[test]
fn a_server_already_running_recovers_a_claim_left_open_at_its_next_tick() {
    block_on!(async {
        let log = Shared::default();
        let (server, _, now, _, _dir) = on_a_store("firing-left", &log).await;
        let made = own_daily(&server).await;
        // Another server's claim, its task's lease long lapsed.
        claim_left(&log, &made).await;
        set(&now, "2026-10-10T11:00:30Z");
        assert_eq!(server.tick().await.unwrap(), Vec::new(), "never run twice");
        assert!(
            interrupted(&log, &made).await,
            "recorded interrupted, no restart needed"
        );
    });
}
