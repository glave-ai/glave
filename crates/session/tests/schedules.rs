//! Schedules through the session API, on a clock the test moves: one-off and cron schedules
//! fire at the right times and once each, each run in a new thread on a worktree of its own;
//! schedules in two projects both fire, and one in a folder that is no repository is skipped;
//! missed runs fire once, never a burst, including in a fresh server; deleting a schedule
//! stops its runs; every run has a budget; and policy limits how many schedules are kept.

use std::path::PathBuf;
use std::sync::Arc;

use glave_event::{At, Opened, ScheduleId, When};
use glave_ports::testing::{ManualClock, TempDir};
use glave_ports::{EventLog, InMemoryLeases, InMemoryLog, Leases, ProjectId};
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
        goal: false,
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
