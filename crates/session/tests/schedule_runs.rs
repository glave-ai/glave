//! A schedule's runs in its own chat, on a clock the test moves: the chat is named for it and
//! each run is a turn there; a run that needs the person waits for them, as a chat does, holds
//! no room under the task limit meanwhile, and stops when stopped, archived or deleted, or as
//! the server stops; a run waiting across a restart keeps its budget; a run due meanwhile is
//! skipped and recorded so once; a run refused otherwise is recorded with its reason, once for
//! the same; the scheduler never wakes at once for a run it would not start; deleting the chat
//! deletes the schedule; and a schedule is made on a copy of a repository or refused, its chat
//! gone when it is not kept.

use glave_event::{At, Outcome, ThreadId, When};
use glave_ports::testing::{ManualClock, TempDir};
use glave_ports::{EventLog, InMemoryLog};
use glave_session::Answering;
use glave_session::protocol::{Answer, Reply, Request, RunStatus, ScheduleInfo, TaskStatus};
use jiff::Timestamp;
use serde_json::json;

use crate::support::*;

fn at(text: &str) -> At {
    At::from(text.parse::<Timestamp>().unwrap())
}

fn set(now: &ManualClock, text: &str) {
    now.set_millis(at(text).timestamp().as_millisecond());
}

/// A server answering `answers`, in Ask first so a write asks, with clients to answer, on a
/// repository of its own in `name`, its clock at 10:03.
async fn asking(
    name: &str,
    answers: Vec<glave_ports::Completion>,
    log: Shared,
) -> (Tested<Shared>, ManualClock, std::path::PathBuf, TempDir) {
    let mut setup = Setup::asking(answers, Answering::Clients);
    setup.clock = ManualClock::at_millis(at("2026-10-10T10:03:00Z").timestamp().as_millisecond());
    let now = setup.clock.clone();
    let (server, _, repo, dir) = setup.build_in_repository(name, log).await;
    (server, now, repo, dir)
}

/// A schedule of `server`'s, called `name`, running `cron` with `budget`.
async fn schedule<L: EventLog + 'static>(
    server: &Tested<L>,
    name: &str,
    cron: &str,
    budget: u64,
) -> ScheduleInfo {
    let request = Request::ScheduleCreate {
        project: Some(scope().project()),
        name: Some(name.into()),
        when: When::Cron(cron.into()),
        zone: None,
        prompt: "check the build".into(),
        budget,
    };
    match server.request(request).await.unwrap() {
        Reply::Schedule { schedule } => schedule,
        other => panic!("a schedule, not {other:?}"),
    }
}

async fn listed<L: EventLog + 'static>(server: &Tested<L>) -> Vec<ScheduleInfo> {
    match server.request(Request::ScheduleList {}).await.unwrap() {
        Reply::Schedules { schedules } => schedules,
        other => panic!("schedules, not {other:?}"),
    }
}

/// How many turns `thread` has begun.
async fn turns<L: EventLog + 'static>(server: &Tested<L>, thread: ThreadId) -> usize {
    let kinds = kinds(server, thread).await;
    kinds.iter().filter(|kind| *kind == "turn.started").count()
}

/// The answers of a run that asks the person before it writes, then says it is done, each
/// model call using 15 tokens.
fn writes_after_asking() -> Vec<glave_ports::Completion> {
    let write = json!({"path": "note.md", "content": "hi"});
    vec![
        costing(answer(None, Some(("write", write))), 10, 5, 1),
        costing(answer(Some("Done."), None), 10, 5, 1),
        costing(answer(Some("Done."), None), 10, 5, 1),
    ]
}

#[test]
fn a_schedule_has_a_chat_of_its_own_named_for_it_and_each_run_is_a_turn_there() {
    block_on!(async {
        let answers = (0..4).map(|_| answer(Some("Done."), None)).collect();
        let (server, now, _, _dir) = asking("runs-chat", answers, Shared::default()).await;
        let made = schedule(&server, "Morning check", "0 11 * * *", 10_000).await;
        let chat = made.thread.unwrap();
        assert_eq!(made.name, "Morning check");
        let Reply::Thread { thread } = server
            .request(Request::ThreadGet { thread: chat })
            .await
            .unwrap()
        else {
            panic!("its chat");
        };
        assert_eq!(thread.title.as_deref(), Some("Morning check"));
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
    });
}

#[test]
fn every_run_opens_its_model_calls_with_the_chat_s_first_prefix() {
    block_on!(async {
        let answers = (0..2).map(|_| answer(Some("Done."), None)).collect();
        let mut setup = Setup::asking(answers, Answering::Clients);
        setup.clock =
            ManualClock::at_millis(at("2026-10-10T10:03:00Z").timestamp().as_millisecond());
        let now = setup.clock.clone();
        let (server, seen, _, _dir) = setup
            .build_in_repository("runs-prefix", Shared::default())
            .await;
        let chat = schedule(&server, "Morning check", "0 11 * * *", 10_000)
            .await
            .thread
            .unwrap();
        for day in ["2026-10-10T11:00:00Z", "2026-10-11T11:00:00Z"] {
            set(&now, day);
            assert_eq!(server.tick().await.unwrap(), vec![chat]);
            settle(&server).await;
        }
        let opening = |request: &glave_ports::ModelRequest| {
            let system = serde_json::to_string(&request.system).unwrap();
            (system, serde_json::to_string(&request.tools).unwrap())
        };
        let requests = seen.lock().unwrap().clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(opening(&requests[1]), opening(&requests[0]), "invariant 27");
    });
}

#[test]
fn a_run_due_while_the_last_waits_for_the_person_is_skipped_and_recorded_once() {
    block_on!(async {
        let (server, now, _, _dir) =
            asking("runs-skip", writes_after_asking(), Shared::default()).await;
        let made = schedule(&server, "Hourly check", "0 * * * *", 10_000).await;
        let chat = made.thread.unwrap();
        set(&now, "2026-10-10T11:00:00Z");
        assert_eq!(server.tick().await.unwrap(), vec![chat]);
        // The run waits for the person, as their chat would.
        let approval = asked_in(&server, chat).await;
        let last = listed(&server).await[0].last.clone().unwrap();
        assert_eq!(
            (last.status, last.due),
            (RunStatus::Running, at("2026-10-10T11:00:00Z"))
        );
        set(&now, "2026-10-10T12:00:00Z");
        assert_eq!(server.tick().await.unwrap(), Vec::new(), "skipped");
        let skipped = listed(&server).await[0].clone();
        let last = skipped.last.clone().unwrap();
        assert_eq!(
            (last.status, last.due),
            (RunStatus::Skipped, at("2026-10-10T12:00:00Z"))
        );
        assert_eq!(last.said, "Skipped: the run before was still going");
        // Still waiting at 13:00: held back, not recorded again.
        set(&now, "2026-10-10T13:00:00Z");
        assert_eq!(server.tick().await.unwrap(), Vec::new());
        let again = listed(&server).await[0].last.clone().unwrap();
        assert_eq!(again.due, at("2026-10-10T12:00:00Z"), "recorded once");
        assert!(
            server.next_due().unwrap() > now_of(&now),
            "it waits for 14:00"
        );
        assert_eq!(turns(&server, chat).await, 1);
        // Answered, the run goes on and ends; the next one runs at its time.
        let decided = Request::ApprovalDecide {
            thread: chat,
            approval,
            decision: Answer::AllowOnce,
        };
        assert!(server.request(decided).await.is_ok());
        settle(&server).await;
        set(&now, "2026-10-10T14:00:00Z");
        assert_eq!(server.tick().await.unwrap(), vec![chat]);
        settle(&server).await;
        assert_eq!(turns(&server, chat).await, 2);
    });
}

fn now_of(now: &ManualClock) -> Timestamp {
    use glave_ports::Clock as _;
    now.now()
}

#[test]
fn a_run_refused_for_another_reason_is_recorded_with_why() {
    block_on!(async {
        let (server, now, _, _dir) = asking("runs-refused", Vec::new(), Shared::default()).await;
        let made = schedule(&server, "Daily check", "0 11 * * *", 10_000).await;
        let chat = made.thread.unwrap();
        server
            .request(Request::ThreadArchive { thread: chat })
            .await
            .unwrap();
        set(&now, "2026-10-10T11:00:00Z");
        assert_eq!(server.tick().await.unwrap(), Vec::new());
        let last = listed(&server).await[0].last.clone().unwrap();
        assert_eq!(last.status, RunStatus::Refused);
        assert_eq!(last.said, "Couldn't start: its chat is archived");
        assert_eq!(
            server.tick().await.unwrap(),
            Vec::new(),
            "once, until its next time"
        );
    });
}

#[test]
fn a_skip_for_another_reason_than_the_last_is_recorded_and_one_for_the_same_is_not() {
    block_on!(async {
        let (server, now, _, _dir) =
            asking("runs-reasons", writes_after_asking(), Shared::default()).await;
        let made = schedule(&server, "Hourly check", "0 * * * *", 10_000).await;
        let chat = made.thread.unwrap();
        set(&now, "2026-10-10T11:00:00Z");
        assert_eq!(server.tick().await.unwrap(), vec![chat]);
        asked_in(&server, chat).await;
        set(&now, "2026-10-10T12:00:00Z");
        server.tick().await.unwrap();
        let going = listed(&server).await[0].last.clone().unwrap();
        assert_eq!(going.status, RunStatus::Skipped);
        // Archived, its waiting run stops; the next is refused for that, another reason.
        server
            .request(Request::ThreadArchive { thread: chat })
            .await
            .unwrap();
        set(&now, "2026-10-10T13:00:00Z");
        server.tick().await.unwrap();
        let refused = listed(&server).await[0].last.clone().unwrap();
        assert_eq!(
            (refused.status, refused.due),
            (RunStatus::Refused, at("2026-10-10T13:00:00Z"))
        );
        set(&now, "2026-10-10T14:00:00Z");
        server.tick().await.unwrap();
        let again = listed(&server).await[0].last.clone().unwrap();
        assert_eq!(
            again.due,
            at("2026-10-10T13:00:00Z"),
            "the same reason once"
        );
    });
}

/// The tasks `server` lists as running, here or elsewhere.
async fn open_tasks<L: EventLog + 'static>(server: &Tested<L>) -> usize {
    (tasks(server).await.iter())
        .filter(|task| task.status != TaskStatus::Finished)
        .count()
}

#[test]
fn stopping_a_run_that_waits_for_the_person_ends_it_and_the_next_one_runs() {
    block_on!(async {
        let (server, now, _, _dir) =
            asking("runs-stop", writes_after_asking(), Shared::default()).await;
        let made = schedule(&server, "Hourly check", "0 * * * *", 10_000).await;
        let chat = made.thread.unwrap();
        set(&now, "2026-10-10T11:00:00Z");
        assert_eq!(server.tick().await.unwrap(), vec![chat]);
        asked_in(&server, chat).await;
        let stopped = server.request(Request::TasksStop { thread: chat }).await;
        assert!(matches!(stopped, Ok(Reply::Done {})), "{stopped:?}");
        settle(&server).await;
        assert_eq!(open_tasks(&server).await, 0, "its task ended");
        let last = listed(&server).await[0].last.clone().unwrap();
        assert_eq!(last.status, RunStatus::Ended);
        assert_eq!(last.said, "Stopped");
        set(&now, "2026-10-10T12:00:00Z");
        assert_eq!(
            server.tick().await.unwrap(),
            vec![chat],
            "not skipped as going"
        );
        settle(&server).await;
    });
}

#[test]
fn archiving_or_deleting_a_chat_whose_run_waits_ends_the_run() {
    block_on!(async {
        let write = json!({"path": "note.md", "content": "hi"});
        let asks = || costing(answer(None, Some(("write", write.clone()))), 10, 5, 1);
        let answers = vec![asks(), asks(), answer(Some("Done."), None)];
        let (server, now, _, _dir) = asking("runs-put-away", answers, Shared::default()).await;
        let archived = schedule(&server, "One", "0 11 * * *", 10_000)
            .await
            .thread
            .unwrap();
        let deleted = schedule(&server, "Two", "0 11 * * *", 10_000)
            .await
            .thread
            .unwrap();
        set(&now, "2026-10-10T11:00:00Z");
        assert_eq!(server.tick().await.unwrap().len(), 2);
        asked_in(&server, archived).await;
        asked_in(&server, deleted).await;
        assert_eq!(open_tasks(&server).await, 2);
        server
            .request(Request::ThreadArchive { thread: archived })
            .await
            .unwrap();
        server
            .request(Request::ThreadDelete { thread: deleted })
            .await
            .unwrap();
        settle(&server).await;
        assert_eq!(open_tasks(&server).await, 0, "neither is left open");
        assert_eq!(
            listed(&server).await.len(),
            1,
            "the deleted chat's schedule went"
        );
    });
}

#[test]
fn a_server_stops_while_a_run_waits_for_the_person() {
    block_on!(async {
        let (server, now, _, _dir) = asking(
            "runs-server-stops",
            writes_after_asking(),
            Shared::default(),
        )
        .await;
        let chat = schedule(&server, "Daily check", "0 11 * * *", 10_000)
            .await
            .thread
            .unwrap();
        set(&now, "2026-10-10T11:00:00Z");
        assert_eq!(server.tick().await.unwrap(), vec![chat]);
        asked_in(&server, chat).await;
        // Its follower is let go, so stopping waits on nothing the person holds.
        server.shutdown().await;
    });
}

#[test]
fn a_run_waiting_for_the_person_counts_toward_no_limit() {
    block_on!(async {
        let write = json!({"path": "note.md", "content": "hi"});
        let answers = vec![
            costing(answer(None, Some(("write", write))), 10, 5, 1),
            answer(Some("Done."), None),
            answer(Some("Done."), None),
        ];
        let mut setup = Setup::asking(answers, Answering::Clients);
        setup.clock =
            ManualClock::at_millis(at("2026-10-10T10:03:00Z").timestamp().as_millisecond());
        setup.limits.max_running = Some(1);
        let now = setup.clock.clone();
        let (server, _, _, _dir) = setup
            .build_in_repository("runs-waiting-limit", Shared::default())
            .await;
        let waiting = schedule(&server, "Asks", "0 11 * * *", 10_000)
            .await
            .thread
            .unwrap();
        set(&now, "2026-10-10T11:00:00Z");
        assert_eq!(server.tick().await.unwrap(), vec![waiting]);
        let approval = asked_in(&server, waiting).await;
        let other = schedule(&server, "Answers", "30 11 * * *", 10_000)
            .await
            .thread
            .unwrap();
        set(&now, "2026-10-10T11:30:00Z");
        assert_eq!(
            server.tick().await.unwrap(),
            vec![other],
            "the waiting run holds no room"
        );
        let denied = Request::ApprovalDecide {
            thread: waiting,
            approval,
            decision: Answer::Deny,
        };
        assert!(server.request(denied).await.is_ok());
        settle(&server).await;
    });
}

#[test]
fn a_schedule_whose_copy_fails_for_another_reason_is_refused_not_run_in_the_folder() {
    block_on!(async {
        let (server, _, _, dir) = asking("runs-no-copy", Vec::new(), Shared::default()).await;
        // Where copies go is a file, so none can be made, though the folder is a repository.
        std::fs::write(dir.join("worktrees"), "not a folder").unwrap();
        assert!(server.request(daily_check()).await.is_err());
        assert_eq!(listed(&server).await, Vec::new());
        assert_eq!(chats(&server).await, 0, "no chat in the folder itself");
    });
}

#[test]
fn a_schedule_that_is_not_kept_leaves_no_chat() {
    block_on!(async {
        let log = Shared::default();
        let (server, _, _, _dir) = asking("runs-not-kept", Vec::new(), log.clone()).await;
        log.fail_appends_of("schedule.created");
        assert!(server.request(daily_check()).await.is_err());
        assert_eq!(chats(&server).await, 0);
    });
}

/// A request to make a schedule checking the build at 11 every day.
fn daily_check() -> Request {
    Request::ScheduleCreate {
        project: Some(scope().project()),
        name: Some("Daily check".into()),
        when: When::Cron("0 11 * * *".into()),
        zone: None,
        prompt: "check the build".into(),
        budget: 10_000,
    }
}

/// How many chats `server` lists in its project.
async fn chats<L: EventLog + 'static>(server: &Tested<L>) -> usize {
    let request = Request::ThreadList {
        project: Some(scope().project()),
        limit: Some(100),
        show: None,
        cursor: None,
    };
    match server.request(request).await.unwrap() {
        Reply::Threads { threads, .. } => threads.len(),
        other => panic!("threads, not {other:?}"),
    }
}

#[test]
fn the_scheduler_never_wakes_at_once_for_a_run_it_would_not_start() {
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.clock =
            ManualClock::at_millis(at("2026-10-10T10:03:00Z").timestamp().as_millisecond());
        setup.limits.max_running = Some(0);
        let now = setup.clock.clone();
        let (server, _, _, _dir) = setup
            .build_in_repository("runs-limit", InMemoryLog::new())
            .await;
        schedule(&server, "Often", "*/5 * * * *", 10_000).await;
        assert_eq!(server.tick().await.unwrap(), Vec::new());
        set(&now, "2026-10-10T10:20:00Z");
        assert_eq!(
            server.tick().await.unwrap(),
            Vec::new(),
            "held back by the limit"
        );
        // Left to the look every minute, or later than now.
        let next = server.next_due();
        assert!(
            next.is_none_or(|next| next > now_of(&now)),
            "{next:?} is not after now"
        );
    });
}

#[test]
fn deleting_a_schedule_s_chat_deletes_the_schedule() {
    block_on!(async {
        let (server, _, _, _dir) = asking("runs-deleted", Vec::new(), Shared::default()).await;
        let made = schedule(&server, "Daily check", "0 11 * * *", 10_000).await;
        let chat = made.thread.unwrap();
        server
            .request(Request::ThreadDelete { thread: chat })
            .await
            .unwrap();
        assert_eq!(listed(&server).await, Vec::new());
    });
}

#[test]
fn a_run_waiting_for_the_person_across_a_restart_keeps_its_budget() {
    block_on!(async {
        let log = Shared::default();
        let (server, now, repo, _dir) =
            asking("runs-restart", writes_after_asking(), log.clone()).await;
        // 20 tokens: the first call's 15 pass, the second's go past.
        let made = schedule(&server, "Careful", "0 11 * * *", 20).await;
        let chat = made.thread.unwrap();
        set(&now, "2026-10-10T11:00:00Z");
        assert_eq!(server.tick().await.unwrap(), vec![chat]);
        let approval = asked_in(&server, chat).await;
        server.shutdown().await;
        drop(server);
        let mut later = Setup::asking(writes_after_asking().split_off(1), Answering::Clients);
        later.clock = now.clone();
        later.worktrees = repo.parent().map(|dir| dir.join("worktrees"));
        let (restarted, _) = later.build_on(&repo, log).await;
        let recovered = restarted.recover_tasks().await.unwrap();
        assert_eq!(recovered.closed, 0, "adopted, not interrupted");
        let decided = Request::ApprovalDecide {
            thread: chat,
            approval,
            decision: Answer::AllowOnce,
        };
        assert!(restarted.request(decided).await.is_ok());
        settle(&restarted).await;
        let ran = tasks(&restarted).await;
        let run = ran.iter().find(|task| task.thread == chat).unwrap();
        assert_eq!(run.outcome, Some(Outcome::LimitReached), "{run:?}");
    });
}
