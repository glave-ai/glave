//! Schedules through the session API, on a clock the test moves: one-off and cron schedules
//! fire at the right times and once each, each run a new turn in the schedule's own chat, named
//! for it, on a worktree of its own; schedules in two projects both fire, and one in a folder
//! that is no repository is refused; missed runs fire once, never a burst, including in a fresh
//! server; a run due while the last waits for the person is skipped and says so; times are read
//! on the clocks of the schedule's zone; pausing stops runs until resuming, which counts from
//! then; deleting a schedule stops its runs and keeps its chat; every run has a budget; and
//! policy limits how many schedules are kept.

use std::path::PathBuf;
use std::sync::Arc;

use glave_event::{At, CallId, Opened, Outcome, ScheduleId, ThreadId, When};
use glave_ports::testing::{CountingEntropy, ManualClock, TempDir};
use glave_ports::{
    Caller, EventLog, Holder, InMemoryLeases, InMemoryLog, Leased, Leases, OrgId, ProjectId,
};
use glave_session::Answering;
use glave_session::protocol::{Answer, LastRun, Reply, Request, ScheduleInfo, code};
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
        name: None,
        when,
        zone: None,
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

fn utc(text: &str) -> At {
    At::from(text.parse::<Timestamp>().unwrap())
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
            listed[0].last,
            Some(LastRun::Ended {
                due: utc("2026-10-10T10:50:00Z"),
                outcome: Outcome::Completed
            })
        );
        assert_eq!(listed[0].next, Some(utc("2026-10-10T11:00:00Z")));
        assert_eq!(listed[0].repeats, "Every 10 minutes");
        let ran = tasks(&server).await;
        assert_eq!(ran.len(), 2);
        assert!(ran.iter().all(|task| task.schedule == Some(made.schedule)));
        assert!(ran.iter().all(|task| task.budget == Some(10_000)));
        assert!(
            ran.iter().all(|task| Some(task.thread) == made.thread),
            "each run a turn in its chat"
        );
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
        let chat = made.thread.unwrap();
        let kept = server.request(Request::ThreadGet { thread: chat }).await;
        assert!(kept.is_ok(), "its chat stays, with its past runs: {kept:?}");
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
fn schedules_in_two_projects_fire_on_worktrees_and_one_with_no_copy_to_work_on_is_refused() {
    block_on!(async {
        let (server, now, repo, _dir) = on_repository("schedule-projects").await;
        let others = repo.with_extension("others");
        let one = make_project(&server, "one", &repository(&others, "one")).await;
        let two = make_project(&server, "two", &repository(&others, "two")).await;
        let plain = others.join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        let none = make_project(&server, "plain", &plain).await;
        let refused = create_in(&server, Some(none), cron("0 11 * * *"), 1_000).await;
        assert_eq!(
            refused.unwrap_err(),
            code::WORKSPACE,
            "no copy for its chat"
        );
        for project in [two, one] {
            create_in(&server, Some(project), cron("0 11 * * *"), 1_000)
                .await
                .unwrap();
        }
        assert_eq!(list(&server).await.len(), 2, "every project's");
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
        assert_eq!(ran_in, want);
        let ran = tasks(&server).await;
        assert_eq!(ran.len(), 2);
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
    block_on!(async {
        let mut setup = setup();
        setup.limits.max_schedules = Some(1);
        let (server, _, _, _dir) = setup
            .build_in_repository("schedule-limits", InMemoryLog::new())
            .await;
        assert_eq!(
            create(&server, cron("* * * * *"), 0).await.unwrap_err(),
            code::INVALID_PARAMS
        );
        assert_eq!(
            create(&server, cron("every day"), 10).await.unwrap_err(),
            code::INVALID_PARAMS
        );
        let nowhere = Request::ScheduleCreate {
            project: None,
            name: None,
            when: cron("0 9 * * *"),
            zone: Some("Mars/Olympus".into()),
            prompt: "check the build".into(),
            budget: 10,
        };
        let refused = server.request(nowhere).await.unwrap_err();
        assert_eq!(refused.code, code::INVALID_PARAMS);
        assert!(refused.message.contains("Europe/Lisbon"), "{refused:?}");
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
    block_on!(async {
        let (server, _, _, _dir) = on_repository("schedule-ledger").await;
        let made = create(&server, cron("0 9 * * *"), 10).await.unwrap();
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
        let listed: Vec<_> = threads.iter().map(|thread| Some(thread.id)).collect();
        assert_eq!(listed, vec![made.thread], "the schedule's chat alone");
        let ledger = ThreadId::new(Timestamp::UNIX_EPOCH, [0; 10]).unwrap();
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
            name: None,
            when: at_time("2026-10-10T10:05:00Z"),
            zone: None,
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
        let runs = async |caller: &Caller| {
            let listed = Request::TasksList { limit: None };
            match server.request_as(caller, listed).await.unwrap() {
                Reply::Tasks { tasks } => (tasks.iter())
                    .map(|task| task.project)
                    .collect::<Vec<ProjectId>>(),
                other => panic!("tasks, not {other:?}"),
            }
        };
        assert_eq!(
            runs(&Caller::default()).await,
            vec![scope().project()],
            "in its organization"
        );
        assert_eq!(runs(&grace).await, Vec::new(), "and in no other");
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

/// `create`, named `name`, in the time zone `zone` when one is given.
async fn create_named<L: EventLog + 'static>(
    server: &Tested<L>,
    name: &str,
    when: When,
    zone: Option<&str>,
) -> ScheduleInfo {
    let request = Request::ScheduleCreate {
        project: Some(scope().project()),
        name: Some(name.into()),
        when,
        zone: zone.map(str::to_owned),
        prompt: "summarize my email".into(),
        budget: 10_000,
    };
    match server.request(request).await.unwrap() {
        Reply::Schedule { schedule } => schedule,
        other => panic!("a schedule, not {other:?}"),
    }
}

/// How many turns `thread` has begun.
async fn turns<L: EventLog + 'static>(server: &Tested<L>, thread: ThreadId) -> usize {
    let kinds = kinds(server, thread).await;
    kinds.iter().filter(|kind| *kind == "turn.started").count()
}

/// The title `thread` shows.
async fn title<L: EventLog + 'static>(server: &Tested<L>, thread: ThreadId) -> Option<String> {
    match server.request(Request::ThreadGet { thread }).await.unwrap() {
        Reply::Thread { thread } => thread.title,
        other => panic!("a thread, not {other:?}"),
    }
}

#[test]
fn a_schedule_has_a_chat_of_its_own_named_for_it_and_each_run_is_a_turn_there() {
    block_on!(async {
        let (server, now, _, _dir) = on_repository("schedule-chat").await;
        let made = create_named(&server, "Morning check", cron("0 11 * * *"), None).await;
        let chat = made.thread.unwrap();
        assert_eq!(made.name, "Morning check");
        assert_eq!(made.zone, "UTC", "no zone given yet");
        assert_eq!(title(&server, chat).await.as_deref(), Some("Morning check"));
        let (opened, _) = opened(&server, chat).await;
        assert!(matches!(opened, Opened::Worktree { .. }), "{opened:?}");
        assert_eq!(
            turns(&server, chat).await,
            0,
            "nothing runs until it is due"
        );
        for day in ["2026-10-10T11:00:00Z", "2026-10-11T11:00:00Z"] {
            set(&now, day);
            assert_eq!(server.tick().await.unwrap(), vec![chat]);
            settle(&server).await;
        }
        assert_eq!(turns(&server, chat).await, 2, "one chat, a turn a day");
        assert_eq!(
            title(&server, chat).await.as_deref(),
            Some("Morning check"),
            "kept"
        );
    });
}

/// The `approval.requested` `thread` waits at, once its turn has asked.
async fn asked_in<L: EventLog + 'static>(server: &Tested<L>, thread: ThreadId) -> CallId {
    for _ in 0..100_000 {
        let asked = events(server, thread).await.into_iter().find_map(|raw| {
            match glave_event::decode(raw).ok()?.body {
                glave_event::Body::ApprovalRequested(asked) => Some(asked.call_id),
                _ => None,
            }
        });
        if let Some(call) = asked {
            return call;
        }
        tokio::task::yield_now().await;
    }
    panic!("the run never asked");
}

#[test]
fn a_run_due_while_the_last_waits_for_the_person_is_skipped_and_says_so() {
    block_on!(async {
        let write = answer(
            None,
            Some((
                "write",
                serde_json::json!({"path": "note.md", "content": "hi"}),
            )),
        );
        let answers = vec![
            write,
            answer(Some("Done."), None),
            answer(Some("Done."), None),
        ];
        let mut setup = Setup::asking(answers, Answering::Clients);
        setup.clock = ManualClock::at_millis(at("2026-10-10T10:03:00Z"));
        let now = setup.clock.clone();
        let (server, _, _, _dir) = setup
            .build_in_repository("schedule-skip", InMemoryLog::new())
            .await;
        let made = create(&server, cron("0 * * * *"), 10_000).await.unwrap();
        let chat = made.thread.unwrap();
        set(&now, "2026-10-10T11:00:00Z");
        assert_eq!(server.tick().await.unwrap(), vec![chat]);
        // The run waits for the person, as their chat would.
        let approval = asked_in(&server, chat).await;
        let eleven = utc("2026-10-10T11:00:00Z");
        assert_eq!(
            list(&server).await[0].last,
            Some(LastRun::Running { due: eleven })
        );
        set(&now, "2026-10-10T12:00:00Z");
        assert_eq!(server.tick().await.unwrap(), Vec::new(), "skipped");
        let skipped = list(&server).await;
        let noon = utc("2026-10-10T12:00:00Z");
        assert_eq!(skipped[0].last, Some(LastRun::Skipped { due: noon }));
        assert_eq!(skipped[0].next, Some(utc("2026-10-10T13:00:00Z")));
        assert_eq!(turns(&server, chat).await, 1);
        // Answered, the run goes on and ends; the next one runs at its time.
        let decided = server
            .request(Request::ApprovalDecide {
                thread: chat,
                approval,
                decision: Answer::AllowOnce,
            })
            .await;
        assert!(decided.is_ok(), "{decided:?}");
        settle(&server).await;
        set(&now, "2026-10-10T13:00:00Z");
        assert_eq!(tick(&server).await, 1);
        assert_eq!(turns(&server, chat).await, 2);
    });
}

#[test]
fn a_schedule_repeats_on_the_clocks_of_the_person_s_time_zone() {
    block_on!(async {
        let (server, now, _, _dir) = on_repository("schedule-zone").await;
        let unknown = Request::ZoneSet {
            zone: "Nowhere/Else".into(),
        };
        let refused = server.request(unknown).await.unwrap_err();
        assert_eq!(refused.code, code::INVALID_PARAMS);
        let given = Request::ZoneSet {
            zone: "America/New_York".into(),
        };
        server.request(given).await.unwrap();
        // 10:03 UTC is 6:03 in New York.
        let made = create_named(&server, "Coffee", cron("0 8 * * *"), None).await;
        assert_eq!(made.zone, "America/New_York");
        assert_eq!(made.repeats, "Every day at 8:00 am");
        assert_eq!(made.next, Some(utc("2026-10-10T12:00:00Z")));
        let tea = create_named(&server, "Tea", cron("0 8 * * *"), Some("Asia/Tokyo")).await;
        assert_eq!(
            tea.next,
            Some(utc("2026-10-10T23:00:00Z")),
            "the zone it names"
        );
        set(&now, "2026-10-10T11:59:00Z");
        assert_eq!(tick(&server).await, 0);
        set(&now, "2026-10-10T12:00:00Z");
        assert_eq!(tick(&server).await, 1);
    });
}

/// `schedule/pause` of `schedule`, pausing it (`paused`) or resuming it, answered.
async fn pause<L: EventLog + 'static>(
    server: &Tested<L>,
    schedule: ScheduleId,
    paused: bool,
) -> ScheduleInfo {
    let request = Request::SchedulePause { schedule, paused };
    match server.request(request).await.unwrap() {
        Reply::Schedule { schedule } => schedule,
        other => panic!("the schedule, not {other:?}"),
    }
}

#[test]
fn a_paused_schedule_starts_no_runs_and_resumed_counts_from_then() {
    block_on!(async {
        let (server, now, _, _dir) = on_repository("schedule-pause").await;
        let schedule = create(&server, cron("0 * * * *"), 10_000)
            .await
            .unwrap()
            .schedule;
        let paused = pause(&server, schedule, true).await;
        assert!(paused.paused);
        assert_eq!((paused.next, server.next_due()), (None, None));
        assert!(pause(&server, schedule, true).await.paused, "as it is");
        set(&now, "2026-10-10T13:30:00Z");
        assert_eq!(tick(&server).await, 0, "paused through three runs");
        let resumed = pause(&server, schedule, false).await;
        assert!(!resumed.paused);
        assert_eq!(resumed.next, Some(utc("2026-10-10T14:00:00Z")));
        assert_eq!(tick(&server).await, 0, "none for the time it was paused");
        set(&now, "2026-10-10T14:00:00Z");
        assert_eq!(tick(&server).await, 1);
    });
}
