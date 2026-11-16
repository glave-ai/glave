//! A schedule's runs in its own chat, on a clock the test moves: the chat is named for it and
//! each run is a turn there; a run that needs the person waits for them, as a chat does; a run
//! due meanwhile is skipped and recorded so once; a run refused otherwise is recorded with why;
//! the scheduler never wakes at once for a run it would not start; and deleting the chat
//! deletes the schedule.

use glave_event::{At, ThreadId, When};
use glave_ports::testing::{ManualClock, TempDir};
use glave_ports::{EventLog, InMemoryLog};
use glave_session::Answering;
use glave_session::protocol::{Answer, Reply, Request, RunStatus, ScheduleInfo};
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
        assert!(last.said.starts_with("Couldn't start: "), "{}", last.said);
        assert_eq!(
            server.tick().await.unwrap(),
            Vec::new(),
            "once, until its next time"
        );
    });
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
        let next = server.next_due().unwrap();
        assert!(next > now_of(&now), "{next} is not after now");
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
