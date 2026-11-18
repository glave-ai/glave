//! Firing every run once: past the ledgers a server keeps in memory, every project's runs are
//! still looked at, each found right after its ledger is read; two servers on one store that
//! both believe they fire start someone else's run once, the one whose claim (its
//! `task.started`) lands, a tick that runs long stops while its lease holds, the next going
//! on, and a claim whose server stopped before the turn began is recorded lost, never run; a schedule made by no one glave can act for, or to run more often than one
//! may, is refused and recorded; an organization has schedules in at most its own share of
//! projects; and a run is never someone else's in more than their own sign-in would be: no
//! sign-in of the owner's, and no Full access, fired or adopted.

use std::sync::Arc;

use glave_event::{
    At, Body, EventId, Fired, Forge, Header, Outcome, Payload, Principal, ScheduleCreated,
    ScheduleId, SchedulePlaced, Seq, Skipped, TaskStarted, ThreadId, When, decode, encode,
};
use glave_ports::computer::{CredentialHost, GitCredential};
use glave_ports::stream::Reserved;
use glave_ports::testing::{FaultyLog, FixedGitCredentials, ManualClock, TempDir, any_secret};
use glave_ports::{
    Caller, EventLog as _, InMemoryLeases, InMemoryProjects, Leases, ProjectId, Projects, Scope,
};
use glave_session::protocol::{Answer, Reply, Request, ScheduleInfo, code};
use jiff::Timestamp;
use serde_json::json;

use crate::schedule_people::refusal_in;
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

/// Two servers on one store: its log, which the first reaches through `faulty`, its projects
/// and its folder, and, where `shared`, its leases, on the test's clock; else each its own.
async fn two_on_a_store(
    name: &str,
    log: &Shared,
    faulty: &FaultyLog,
    shared: bool,
) -> (TempDir, Tested<FaultyLog>, Tested<Shared>, ManualClock) {
    let projects: Arc<dyn Projects> = Arc::new(InMemoryProjects::new());
    let mut first = setup();
    first.projects = Some(Arc::clone(&projects));
    let now = first.clock.clone();
    let leases =
        shared.then(|| -> Arc<dyn Leases> { Arc::new(InMemoryLeases::new(Arc::new(now.clone()))) });
    first.leases = leases.as_ref().map(Arc::clone);
    let (one, _, repo, dir) = first.build_in_repository(name, faulty.clone()).await;
    let mut second = setup();
    second.clock = now.clone();
    second.projects = Some(projects);
    second.leases = leases;
    second.seed = 2;
    let (two, _) = second.build_on(&repo, log.clone()).await;
    (dir, one, two, now)
}

#[test]
fn two_servers_that_both_fire_start_someone_else_s_run_once() {
    block_on!(async {
        let log = Shared::default();
        let faulty = FaultyLog::over(Arc::clone(&log.0));
        // Each server keeps leases of its own, so both believe they fire the store's schedules.
        let (_dir, one, two, now) = two_on_a_store("firing-twice", &log, &faulty, false).await;
        let grace = elsewhere();
        let made = daily(&one, &grace).await.unwrap();
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
        settle_for(&one, &grace).await;
        settle_for(&two, &grace).await;
        assert_eq!(
            runs_in(&log, made.project).await.len(),
            1,
            "one claim landed; the other read it and wrote none"
        );
    });
}

#[test]
fn a_tick_that_runs_long_stops_before_its_lease_could_lapse_and_the_next_goes_on() {
    block_on!(async {
        let log = Shared::default();
        let faulty = FaultyLog::over(Arc::clone(&log.0));
        // One lease for both, as a store's.
        let (_dir, one, two, now) = two_on_a_store("firing-long", &log, &faulty, true).await;
        let grace = elsewhere();
        let made = [own_daily(&one).await, daily(&one, &grace).await.unwrap()];
        set(&now, "2026-10-10T11:00:00Z");
        // The first run's claim is held as it is written while 100 seconds pass, past the share
        // of a tick an organization takes; the other server ticks meanwhile.
        faulty.hold("task.started");
        let racing = async {
            faulty.held().await;
            set(&now, "2026-10-10T11:01:40Z");
            let fired = two.tick().await;
            faulty.release();
            fired
        };
        let (from_one, from_two) = tokio::join!(one.tick(), racing);
        assert_eq!(
            from_one.unwrap().len(),
            1,
            "it stopped after the run it was starting"
        );
        assert_eq!(
            from_two.unwrap(),
            Vec::new(),
            "the other never fires while it ticks"
        );
        // The next tick goes on where it stopped.
        set(&now, "2026-10-10T11:01:45Z");
        assert_eq!(one.tick().await.unwrap().len(), 1);
        settle(&one).await;
        settle_for(&one, &grace).await;
        for schedule in &made {
            assert_eq!(
                runs_in(&log, schedule.project).await.len(),
                1,
                "each run once"
            );
        }
    });
}

/// A daily schedule at 11:00 in the server's own project, which a server started since
/// recovers too.
async fn own_daily<L: glave_ports::EventLog + 'static>(server: &Tested<L>) -> ScheduleInfo {
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

/// The `n`th id of the kind `kind` stands for, a UUID v7.
fn v7(kind: u128, n: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(0x0192_f5c0_0000_7000_8000_0000_0000_0000 + (kind << 80) + n)
}

/// A chat no one made: the `n`th.
fn missing_chat(n: u128) -> ThreadId {
    ThreadId::try_from(v7(4, n)).unwrap()
}

/// The `n`th schedule, made at 10:00, that runs at 11:00 every day into `thread`, made by `by`.
pub(crate) fn made_by(n: u128, thread: ThreadId, by: Option<Principal>) -> ScheduleCreated {
    ScheduleCreated {
        schedule: ScheduleId::try_from(v7(1, n)).unwrap(),
        name: "Daily check".into(),
        when: When::Cron("0 11 * * *".into()),
        zone: "UTC".into(),
        prompt: "check the build".into(),
        budget: 5_000,
        thread: Some(thread),
        from: None,
        by,
    }
}

#[test]
fn past_the_ledgers_a_server_keeps_every_project_s_runs_are_still_looked_at() {
    block_on!(async {
        let log = Shared::default();
        let first = setup();
        let now = first.clock.clone();
        let (server, _, _, _dir) = first.build_in_repository("firing-many", log.clone()).await;
        // More projects with schedules than a server keeps ledgers of, written straight into
        // the store, each schedule's chat missing so each run is refused and recorded.
        let org = scope().org();
        let (index, ledger) = (Reserved::Scheduled.id(), Reserved::Tasks.id());
        let projects: Vec<ProjectId> = (0..1_030).map(|n| ProjectId::from_uuid(v7(3, n))).collect();
        for (n, project) in (0u64..).zip(&projects) {
            let placed = SchedulePlaced {
                project: project.as_uuid(),
            };
            append(&log, &Scope::of_organization(org), index, n + 1, &placed).await;
            let created = made_by(n.into(), missing_chat(n.into()), None);
            append(&log, &Scope::glaves_own(org, *project), ledger, 1, &created).await;
        }
        set(&now, "2026-10-10T11:00:00Z");
        assert_eq!(server.tick().await.unwrap(), Vec::new());
        for project in &projects {
            let refused = refusal_in(&log, *project).await;
            assert!(!refused.is_empty(), "{project}'s run was never looked at");
        }
    });
}

#[test]
fn reads_slower_than_an_organization_s_share_still_start_the_first_project_s_runs() {
    block_on!(async {
        let log = Shared::default();
        let faulty = FaultyLog::over(Arc::clone(&log.0));
        let first = setup();
        let now = first.clock.clone();
        let (server, _, _, _dir) = first
            .build_in_repository("firing-slow", faulty.clone())
            .await;
        let chat = start(&server, false).await;
        // A schedule another server wrote, so this one reads its ledger as it ticks.
        let made = made_by(1, chat, server.person().map(Principal::person));
        append(&log, &scope(), Reserved::Tasks.id(), 1, &made).await;
        set(&now, "2026-10-10T11:00:00Z");
        // That read takes longer than an organization's share of a tick.
        faulty.hold_read("schedule.created");
        let slow = async {
            faulty.held().await;
            set(&now, "2026-10-10T11:00:20Z");
            faulty.release();
        };
        let (started, ()) = tokio::join!(server.tick(), slow);
        assert_eq!(
            started.unwrap(),
            vec![chat],
            "the first project read still starts"
        );
        settle(&server).await;
    });
}

#[test]
fn a_schedule_made_to_run_more_often_than_one_may_is_refused_at_its_time() {
    block_on!(async {
        let log = Shared::default();
        let first = setup();
        let now = first.clock.clone();
        let (server, _, _, _dir) = first.build_in_repository("firing-often", log.clone()).await;
        let chat = start(&server, false).await;
        let mut often = made_by(2, chat, None);
        often.when = When::Cron("* * * * *".into());
        append(&log, &scope(), Reserved::Tasks.id(), 1, &often).await;
        set(&now, "2026-10-10T11:00:00Z");
        assert_eq!(server.tick().await.unwrap(), Vec::new());
        assert_eq!(refusal_in(&log, scope().project()).await, "TOO_OFTEN");
    });
}

#[test]
fn an_organization_has_schedules_in_at_most_its_own_share_of_projects() {
    block_on!(async {
        let log = Shared::default();
        let (server, _, _, _dir) = setup()
            .build_in_repository("firing-share", log.clone())
            .await;
        // As many of its projects listed as one organization may have.
        let org = scope().org();
        let index = (Scope::of_organization(org), Reserved::Scheduled.id());
        for n in 0..10_000u64 {
            let placed = SchedulePlaced {
                project: v7(5, n.into()),
            };
            append(&log, &index.0, index.1, n + 1, &placed).await;
        }
        assert_eq!(
            daily(&server, &Caller::default()).await.unwrap_err(),
            code::LIMIT,
            "a project of its own not listed yet"
        );
    });
}

#[test]
fn a_schedule_made_by_no_one_glave_can_act_for_is_refused_and_recorded() {
    block_on!(async {
        let log = Shared::default();
        let first = setup();
        let now = first.clock.clone();
        let (server, _, _, _dir) = first
            .build_in_repository("firing-no-one", log.clone())
            .await;
        let project = scope().project();
        // Made by glave itself, and by no one named, which no server reads as its own person.
        let created = made_by(1, missing_chat(1), Some(Principal::glave()));
        append(&log, &scope(), Reserved::Tasks.id(), 1, &created).await;
        let unnamed = made_by(2, missing_chat(2), None);
        append(&log, &scope(), Reserved::Tasks.id(), 2, &unnamed).await;
        set(&now, "2026-10-10T11:00:00Z");
        assert_eq!(server.tick().await.unwrap(), Vec::new());
        let refused = (ledger_of(&log, project).await.into_iter())
            .filter(|body| {
                matches!(body, Body::ScheduleSkipped(skipped)
                    if skipped.why == Skipped::Refused { reason: "MAKER_UNKNOWN".into() })
            })
            .count();
        assert_eq!(
            refused, 2,
            "both refused, as made by no one glave can act for"
        );
    });
}

/// Whether the model was last asked with the owner's GitHub said signed in.
fn told_signed_in(seen: &Seen) -> bool {
    let asked = seen.lock().unwrap();
    let system = &asked.last().unwrap().system;
    serde_json::to_string(system)
        .unwrap()
        .contains("`gh` is signed in for github.com")
}

/// How many times `thread` asked the person to approve something.
async fn asked<L: glave_ports::EventLog + 'static>(server: &Tested<L>, thread: ThreadId) -> usize {
    (events_as(server, &elsewhere(), thread).await.into_iter())
        .filter(|raw| {
            matches!(
                decode(raw.clone()).unwrap().body,
                Body::ApprovalRequested(_)
            )
        })
        .count()
}

#[test]
fn someone_else_s_run_fired_or_adopted_has_none_of_the_owner_s_sign_ins_nor_full_access() {
    block_on!(async {
        let log = Shared::default();
        let projects: Arc<dyn Projects> = Arc::new(InMemoryProjects::new());
        // The owner's GitHub is signed in on the server's computer, whose mode is Full access.
        let owner_s = || {
            let host = CredentialHost {
                url: "https://github.com".into(),
                forge: Forge::Github,
            };
            let credential = GitCredential::new("x-access-token", any_secret()).unwrap();
            Arc::new(FixedGitCredentials::new(vec![host], credential))
        };
        let write =
            |path: &str| answer(None, Some(("write", json!({"path": path, "content": "x"}))));
        let mut first = Setup::new(vec![write("a.md")]);
        first.clock = setup().clock;
        first.projects = Some(Arc::clone(&projects));
        first.credentials = Some(owner_s());
        let now = first.clock.clone();
        let (server, seen, repo, _dir) = first
            .build_in_repository("firing-sandboxed", log.clone())
            .await;
        let grace = elsewhere();
        let chat = daily(&server, &grace).await.unwrap().thread.unwrap();
        set(&now, "2026-10-10T11:00:00Z");
        assert_eq!(server.tick().await.unwrap(), vec![chat]);
        let approval = asked_in_as(&server, &grace, chat).await;
        assert!(
            !told_signed_in(&seen),
            "fired: none of the owner's sign-ins"
        );
        assert_eq!(
            asked(&server, chat).await,
            1,
            "fired: its write asks, as no Full access"
        );
        server.shutdown().await;
        drop(server);
        // The next server adopts the waiting run and goes on with it once she answers.
        let mut later = Setup::new(vec![write("b.md"), answer(Some("Done."), None)]);
        later.clock = now.clone();
        later.projects = Some(projects);
        later.credentials = Some(owner_s());
        later.worktrees = repo.parent().map(|dir| dir.join("worktrees"));
        let (restarted, seen) = later.build_on(&repo, log).await;
        assert_eq!(
            restarted.recover_tasks().await.unwrap().closed,
            0,
            "adopted"
        );
        let decided = Request::ApprovalDecide {
            thread: chat,
            approval,
            decision: Answer::AllowOnce,
        };
        restarted.request_as(&grace, decided).await.unwrap();
        for _ in 0..100_000 {
            if asked(&restarted, chat).await == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(
            asked(&restarted, chat).await,
            2,
            "adopted: its next write asks too"
        );
        assert!(
            !told_signed_in(&seen),
            "adopted: none of the owner's sign-ins"
        );
        restarted.shutdown().await;
    });
}
