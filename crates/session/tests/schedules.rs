//! Schedules through the session API, on a clock the test moves: one-off and cron schedules
//! fire at the right times and once each, each run in a new thread on a worktree of its own;
//! schedules in two projects both fire, and one in a folder that is no repository is skipped;
//! missed runs fire once, never a burst, including in a fresh server; deleting a schedule, or
//! removing its project, stops its runs; every run has a budget; and policy limits how many
//! schedules are kept.

use std::path::PathBuf;
use std::sync::Arc;

use glave_event::{
    At, Body, EventId, Header, Opened, Payload, Principal, PrincipalId, ScheduleCreated,
    ScheduleDeleted, ScheduleId, Seq, When, decode, encode,
};
use glave_ports::stream::Reserved;
use glave_ports::testing::{CountingEntropy, FaultyLog, ManualClock, TempDir};
use glave_ports::{
    Caller, EventLog, Holder, InMemoryLeases, InMemoryLog, InMemoryProjects, Leased, Leases, OrgId,
    Project, ProjectId, ProjectKind, Projects, Scope,
};
use glave_session::protocol::{Reply, Request, ScheduleInfo, code};
use jiff::Timestamp;

use crate::support::*;

fn at(text: &str) -> i64 {
    text.parse::<Timestamp>().unwrap().as_millisecond()
}

fn set(now: &ManualClock, text: &str) {
    now.set_millis(at(text));
}

/// A server with plenty of short answers, each run taking one, on a clock standing at 10:03
/// until the test moves it.
fn setup() -> Setup {
    let mut setup = Setup::new((0..20).map(|_| answer(Some("Done."), None)).collect());
    setup.clock = ManualClock::at_millis(at("2026-10-10T10:03:00Z"));
    setup
}

/// `setup()` on a repository of its own in the fresh folder `name`: the server, its clock and
/// the repository.
async fn on_repository(name: &str) -> (Tested, ManualClock, PathBuf, TempDir) {
    let setup = setup();
    let now = setup.clock.clone();
    let (server, _, repo, dir) = setup.build_in_repository(name, InMemoryLog::new()).await;
    (server, now, repo, dir)
}

async fn create<L: EventLog + 'static>(
    server: &Tested<L>,
    when: When,
    budget: u64,
) -> Result<ScheduleInfo, i64> {
    create_in(server, Some(scope().project()), when, budget).await
}

async fn create_in<L: EventLog + 'static>(
    server: &Tested<L>,
    project: Option<ProjectId>,
    when: When,
    budget: u64,
) -> Result<ScheduleInfo, i64> {
    let request = Request::ScheduleCreate {
        project,
        when,
        prompt: "check the build".into(),
        budget,
    };
    match server.request(request).await {
        Ok(Reply::Schedule { schedule }) => Ok(schedule),
        Ok(other) => panic!("a schedule, not {other:?}"),
        Err(error) => Err(error.code),
    }
}

async fn list<L: EventLog + 'static>(server: &Tested<L>) -> Vec<ScheduleInfo> {
    match server.request(Request::ScheduleList {}).await.unwrap() {
        Reply::Schedules { schedules } => schedules,
        other => panic!("schedules, not {other:?}"),
    }
}

async fn tick<L: EventLog + 'static>(server: &Tested<L>) -> usize {
    let fired = server.tick().await.unwrap();
    settle(server).await;
    fired.len()
}

fn cron(text: &str) -> When {
    When::Cron(text.into())
}

#[test]
fn a_cron_schedule_fires_when_due_once_each_time_and_missed_runs_fire_once() {
    block_on!(async {
        let (server, now, _, _dir) = on_repository("schedule-cron").await;
        let made = create(&server, cron("*/10 * * * *"), 10_000).await.unwrap();
        assert_eq!(
            made.next,
            Some(At::from(
                "2026-10-10T10:10:00Z".parse::<Timestamp>().unwrap()
            ))
        );
        assert_eq!(tick(&server).await, 0, "not due at 10:03");
        set(&now, "2026-10-10T10:09:59Z");
        assert_eq!(tick(&server).await, 0);
        set(&now, "2026-10-10T10:10:00Z");
        assert_eq!(tick(&server).await, 1, "due at 10:10");
        assert_eq!(tick(&server).await, 0, "once");
        // 10:20 to 10:50 are missed; one run, never four.
        set(&now, "2026-10-10T10:55:00Z");
        assert_eq!(tick(&server).await, 1);
        assert_eq!(tick(&server).await, 0);
        let listed = list(&server).await;
        assert_eq!(
            listed[0].last.map(|a| a.to_string()).as_deref(),
            Some("2026-10-10T10:50:00.000Z")
        );
        assert_eq!(
            listed[0].next.map(|a| a.to_string()).as_deref(),
            Some("2026-10-10T11:00:00.000Z")
        );
        let ran = tasks(&server).await;
        assert_eq!(ran.len(), 2);
        assert!(ran.iter().all(|task| task.schedule == Some(made.schedule)));
        assert!(ran.iter().all(|task| task.budget == Some(10_000)));
        // Deleted: no more runs.
        server
            .request(Request::ScheduleDelete {
                schedule: made.schedule,
            })
            .await
            .unwrap();
        set(&now, "2026-10-10T12:00:00Z");
        assert_eq!(tick(&server).await, 0);
        assert_eq!(list(&server).await, Vec::new());
        let again = server
            .request(Request::ScheduleDelete {
                schedule: made.schedule,
            })
            .await
            .unwrap_err();
        assert_eq!(again.code, code::NO_SCHEDULE);
    });
}

#[test]
fn a_one_off_schedule_fires_once_at_its_time() {
    block_on!(async {
        let (server, now, _, _dir) = on_repository("schedule-once").await;
        let when = When::At(At::from(
            "2026-10-10T11:00:00Z".parse::<Timestamp>().unwrap(),
        ));
        create(&server, when, 5_000).await.unwrap();
        set(&now, "2026-10-10T10:59:59Z");
        assert_eq!(tick(&server).await, 0);
        set(&now, "2026-10-10T11:00:00Z");
        assert_eq!(tick(&server).await, 1);
        set(&now, "2026-10-11T11:00:00Z");
        assert_eq!(tick(&server).await, 0);
        let listed = list(&server).await;
        assert_eq!(listed[0].next, None, "it has run");
        assert_eq!(server.next_due(), None);
    });
}

#[test]
fn schedules_in_two_projects_fire_on_worktrees_and_one_with_no_copy_to_work_on_is_skipped() {
    block_on!(async {
        let (server, now, repo, _dir) = on_repository("schedule-projects").await;
        let others = repo.with_extension("others");
        let one = make_project(&server, "one", &repository(&others, "one")).await;
        let two = make_project(&server, "two", &repository(&others, "two")).await;
        let plain = others.join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        let none = make_project(&server, "plain", &plain).await;
        for project in [none, two, one] {
            create_in(&server, Some(project), cron("0 11 * * *"), 1_000)
                .await
                .unwrap();
        }
        assert_eq!(list(&server).await.len(), 3, "every project's");
        set(&now, "2026-10-10T11:00:00Z");
        let fired = server.tick().await.unwrap();
        settle(&server).await;
        let mut ran_in = Vec::new();
        for thread in &fired {
            let Reply::Thread { thread: summary } = server
                .request(Request::ThreadGet { thread: *thread })
                .await
                .unwrap()
            else {
                panic!("a thread");
            };
            ran_in.push(summary.project);
            let (opened, _) = opened(&server, *thread).await;
            assert!(matches!(opened, Opened::Worktree { .. }), "{opened:?}");
        }
        ran_in.sort();
        let mut want = vec![one, two];
        want.sort();
        assert_eq!(ran_in, want, "the folder that is no repository has no copy");
        let ran = tasks(&server).await;
        assert_eq!(ran.len(), 2);
        // The skipped run waits for the schedule's next time, as the others do.
        assert_eq!(tick(&server).await, 0);
        set(&now, "2026-10-11T11:00:00Z");
        assert_eq!(tick(&server).await, 2);
    });
}

#[test]
fn removing_a_project_deletes_its_schedules_and_starts_no_more_runs_there() {
    block_on!(async {
        let (server, now, repo, _dir) = on_repository("schedule-removed").await;
        let others = repo.with_extension("others");
        let removed = make_project(&server, "removed", &repository(&others, "removed")).await;
        let kept = make_project(&server, "kept", &repository(&others, "kept")).await;
        create_in(&server, Some(removed), cron("0 11 * * *"), 1_000)
            .await
            .unwrap();
        let stays = create_in(&server, Some(kept), cron("0 11 * * *"), 1_000)
            .await
            .unwrap();
        let done = server
            .request(Request::ProjectRemove { project: removed })
            .await;
        assert!(matches!(done, Ok(Reply::Done {})), "{done:?}");
        let listed: Vec<ScheduleId> = list(&server).await.iter().map(|s| s.schedule).collect();
        assert_eq!(listed, [stays.schedule], "the removed project's is deleted");

        set(&now, "2026-10-10T11:00:00Z");
        let fired = server.tick().await.unwrap();
        settle(&server).await;
        let [thread] = fired[..] else {
            panic!("one run, not {fired:?}");
        };
        let Reply::Thread { thread: summary } =
            server.request(Request::ThreadGet { thread }).await.unwrap()
        else {
            panic!("a thread");
        };
        assert_eq!(summary.project, kept, "none in the removed project");
    });
}

/// Writes `payload` at `seq` to the task ledger in `ledger`, as another server would.
async fn written<T: Payload>(log: &InMemoryLog, ledger: &Scope, seq: u64, payload: &T) {
    let mut random = [5; 10];
    random[2..].copy_from_slice(&seq.to_be_bytes());
    let at = At::from(Timestamp::from_millisecond(at("2026-10-10T10:00:00Z")).unwrap());
    let id = EventId::new(at.timestamp(), random).unwrap();
    let (thread, seq) = (Reserved::Tasks.id(), Seq::new(seq).unwrap());
    let raw = encode(
        Header {
            id,
            thread,
            seq,
            at,
        },
        payload,
    )
    .unwrap();
    log.append(ledger, &[raw]).await.unwrap();
}

/// Runs the task ledger in `ledger` past one page of a read: 499 more schedules, then, on the
/// second page, the deletion of one long gone.
async fn past_a_page(log: &InMemoryLog, ledger: &Scope) {
    let tasks = Reserved::Tasks.id();
    let next = log
        .read(ledger, tasks, None, usize::MAX)
        .await
        .unwrap()
        .len();
    let next = u64::try_from(next).unwrap();
    for n in 1..=500u64 {
        let schedule = ScheduleId::try_from(uuid::Uuid::from_u128(
            0x0192_f5c1_0000_7000_8000_0000_0001_0000 + u128::from(n),
        ))
        .unwrap();
        if n < 500 {
            let created = ScheduleCreated {
                schedule,
                when: cron("0 11 * * *"),
                prompt: "check the build".into(),
                budget: 1_000,
                by: None,
            };
            written(log, ledger, next + n, &created).await;
        } else {
            let gone = ScheduleDeleted { schedule, by: None };
            written(log, ledger, next + n, &gone).await;
        }
    }
}

/// Keeps `count` projects of `person`'s in `projects`, the `batch`th lot of them.
async fn many_projects(projects: &dyn Projects, person: PrincipalId, batch: u128, count: u32) {
    for n in 0..count {
        let project = Project {
            id: ProjectId::from_uuid(uuid::Uuid::from_u128(
                0x0192_f5c3_0000_7000_8000_0000_0000_0000 + (batch << 32) + u128::from(n),
            )),
            kind: ProjectKind::Project,
            name: format!("project {n}"),
            owner: Principal::person(person),
            created_at: At::from(Timestamp::UNIX_EPOCH),
            updated_at: At::from(Timestamp::UNIX_EPOCH),
        };
        projects.create(scope().org(), &project).await.unwrap();
    }
}

#[test]
fn removing_a_project_deletes_every_schedule_there_though_its_ledger_is_forgotten_meanwhile() {
    block_on!(async {
        let log = Arc::new(InMemoryLog::new());
        let faulty = FaultyLog::over(Arc::clone(&log));
        let projects: Arc<dyn Projects> = Arc::new(InMemoryProjects::new());
        let mut first = setup();
        first.projects = Some(Arc::clone(&projects));
        let (server, _, repo, _dir) = first
            .build_in_repository("schedule-removed-race", faulty.clone())
            .await;
        let others = repo.with_extension("others");
        let removed = make_project(&server, "removed", &repository(&others, "removed")).await;
        create_in(&server, Some(removed), cron("0 11 * * *"), 1_000)
            .await
            .unwrap();
        let ledger = Scope::glaves_own(scope().org(), removed);
        past_a_page(&log, &ledger).await;
        // More projects than a server keeps ledgers of: the person's, and Grace's.
        let grace = elsewhere();
        many_projects(projects.as_ref(), server.person().unwrap(), 0, 1_000).await;
        many_projects(projects.as_ref(), grace.person.unwrap(), 1, 100).await;
        // The removal's read of the ledger is held at its second page while both list their
        // schedules, which reads every project's ledger, forgetting the rest.
        faulty.hold_read("schedule.deleted");
        let removal = server.request(Request::ProjectRemove { project: removed });
        let forgetting = async {
            faulty.held().await;
            server.request(Request::ScheduleList {}).await.unwrap();
            server
                .request_as(&grace, Request::ScheduleList {})
                .await
                .unwrap();
            faulty.release();
        };
        let (done, ()) = tokio::join!(removal, forgetting);
        assert!(matches!(done, Ok(Reply::Done {})), "{done:?}");
        let deleted = (log
            .read(&ledger, Reserved::Tasks.id(), None, usize::MAX)
            .await
            .unwrap()
            .into_iter())
        .filter(|raw| matches!(decode(raw.clone()).unwrap().body, Body::ScheduleDeleted(_)))
        .count();
        assert_eq!(
            deleted, 501,
            "every schedule there, read whole, and the one long gone"
        );
    });
}

#[test]
fn a_fresh_server_runs_what_was_missed_while_none_ran_once() {
    block_on!(async {
        let log = Shared::default();
        let first = setup();
        let now = first.clock.clone();
        let (server, _, repo, _dir) = first
            .build_in_repository("schedule-restart", log.clone())
            .await;
        create(&server, cron("0 * * * *"), 10_000).await.unwrap();
        server.shutdown().await;
        drop(server);
        // Down for a day: 24 runs missed.
        let mut later = setup();
        later.clock = now.clone();
        later.worktrees = repo.parent().map(|dir| dir.join("worktrees"));
        set(&now, "2026-10-11T10:30:00Z");
        let (restarted, _) = later.build_on(&repo, log).await;
        let recovered = restarted.recover_tasks().await.unwrap();
        assert_eq!(recovered, glave_session::Repaired::default());
        assert_eq!(tick(&restarted).await, 1);
        assert_eq!(tick(&restarted).await, 0);
        set(&now, "2026-10-11T11:00:00Z");
        assert_eq!(tick(&restarted).await, 1);
    });
}

#[test]
fn schedules_need_a_budget_a_valid_time_and_room_under_policy() {
    let dir = folder("schedule-limits");
    block_on!(async {
        let mut setup = setup();
        setup.limits.max_schedules = Some(1);
        let (server, _) = setup.build(&dir).await;
        assert_eq!(
            create(&server, cron("* * * * *"), 0).await.unwrap_err(),
            code::INVALID_PARAMS
        );
        assert_eq!(
            create(&server, cron("every day"), 10).await.unwrap_err(),
            code::INVALID_PARAMS
        );
        create(&server, cron("0 9 * * 1-5"), 10).await.unwrap();
        assert_eq!(
            create(&server, cron("0 10 * * *"), 10).await.unwrap_err(),
            code::LIMIT
        );
        let unknown = ScheduleId::new(Timestamp::UNIX_EPOCH, [7; 10]).unwrap();
        let missing = server
            .request(Request::ScheduleDelete { schedule: unknown })
            .await
            .unwrap_err();
        assert_eq!(missing.code, code::NO_SCHEDULE);
    });
}

#[test]
fn the_task_ledger_is_never_listed_or_read_as_a_thread() {
    let dir = folder("schedule-ledger");
    block_on!(async {
        let (server, _) = setup().build(&dir).await;
        create(&server, cron("0 9 * * *"), 10).await.unwrap();
        let Reply::Threads { threads, .. } = server
            .request(Request::ThreadList {
                project: Some(scope().project()),
                limit: None,
                show: None,
                cursor: None,
            })
            .await
            .unwrap()
        else {
            panic!("threads");
        };
        assert_eq!(threads, Vec::new());
        let ledger = glave_event::ThreadId::new(Timestamp::UNIX_EPOCH, [0; 10]).unwrap();
        let read = server
            .request(Request::ThreadEvents {
                thread: ledger,
                after: None,
                limit: None,
            })
            .await
            .unwrap_err();
        assert_eq!(read.code, code::NO_THREAD);
    });
}

/// One server of the organization fires its schedules, the holder of their lease: a second
/// server on the store fires none while the first holds it, and takes over once it stops.
#[test]
fn schedules_fire_from_one_server_at_a_time() {
    block_on!(async {
        let log = Shared::default();
        let first = setup();
        let now = first.clock.clone();
        let leases = Arc::new(InMemoryLeases::new(Arc::new(now.clone())));
        let mut first = first;
        first.leases = Some(Arc::clone(&leases) as Arc<dyn Leases>);
        let (one, _, repo, _dir) = first
            .build_in_repository("schedule-lease", log.clone())
            .await;
        let mut second = setup();
        second.clock = now.clone();
        second.seed = 2;
        second.settings.person = one.person();
        second.projects = Some(Arc::clone(one.projects()));
        second.leases = Some(Arc::clone(&leases) as Arc<dyn Leases>);
        second.worktrees = repo.parent().map(|dir| dir.join("worktrees"));
        let (two, _) = second.build_on(&repo, log).await;
        let at_time = |text: &str| When::At(At::from(text.parse::<Timestamp>().unwrap()));
        create(&one, at_time("2026-10-10T10:05:00Z"), 5_000)
            .await
            .unwrap();
        create(&one, at_time("2026-10-10T10:07:00Z"), 5_000)
            .await
            .unwrap();
        // Nothing is due at 10:03: the first to look takes the schedules' lease.
        assert_eq!(tick(&one).await, 0);
        set(&now, "2026-10-10T10:05:00Z");
        assert_eq!(tick(&two).await, 0, "the first server fires them");
        assert_eq!(tick(&one).await, 1);
        // The first stops, letting go: the second fires the next run.
        one.shutdown().await;
        set(&now, "2026-10-10T10:07:00Z");
        assert_eq!(tick(&two).await, 1);
        assert_eq!(tick(&two).await, 0, "each run once");
    });
}

/// One server looks after the schedules of every organization its callers work in, each under
/// that organization's own scheduler lease, and a run lands in its schedule's organization:
/// someone in another organization sees nothing of it. A run for someone signed in from
/// elsewhere does not start yet (#410), in any organization.
#[test]
fn one_server_keeps_each_organizations_schedules_under_its_own_lease() {
    block_on!(async {
        let mut first = setup();
        let now = first.clock.clone();
        let leases = Arc::new(InMemoryLeases::new(Arc::new(now.clone())));
        first.leases = Some(Arc::clone(&leases) as Arc<dyn Leases>);
        let (server, _, _, _dir) = first
            .build_in_repository("schedule-orgs", InMemoryLog::new())
            .await;
        let third = OrgId::from_uuid(uuid::Uuid::from_u128(3));
        let grace = Caller {
            org: Some(third),
            ..elsewhere()
        };
        let at_time = |text: &str| When::At(At::from(text.parse::<Timestamp>().unwrap()));
        create(&server, at_time("2026-10-10T10:05:00Z"), 5_000)
            .await
            .unwrap();
        let theirs = Request::ScheduleCreate {
            project: None,
            when: at_time("2026-10-10T10:05:00Z"),
            prompt: "check the build".into(),
            budget: 5_000,
        };
        let made = server.request_as(&grace, theirs).await;
        assert!(made.is_ok(), "{made:?}");
        set(&now, "2026-10-10T10:05:00Z");
        assert_eq!(
            tick(&server).await,
            1,
            "the run of the person on this computer"
        );
        let runs = async |caller: &Caller, project: Option<ProjectId>| {
            let listed = Request::ThreadList {
                project,
                limit: None,
                show: None,
                cursor: None,
            };
            match server.request_as(caller, listed).await.unwrap() {
                Reply::Threads { threads, .. } => threads.len(),
                other => panic!("threads, not {other:?}"),
            }
        };
        let project = Some(scope().project());
        assert_eq!(
            runs(&Caller::default(), project).await,
            1,
            "in its organization"
        );
        assert_eq!(runs(&grace, None).await, 0, "and in no other");
        // The server holds each organization's scheduler lease apart.
        let entropy = CountingEntropy::starting_at(0x7e);
        let other = Holder::new(&now, &entropy).unwrap();
        for org in [third, scope().org()] {
            let taken = leases
                .acquire(
                    org,
                    Leased::Scheduler,
                    other,
                    std::time::Duration::from_secs(60),
                )
                .await
                .unwrap();
            assert!(
                taken.is_none(),
                "each organization's lease is the first server's"
            );
        }
    });
}
