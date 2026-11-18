//! A schedule's runs in its own chat, on a clock the test moves: the chat is named for it and
//! each run is a turn there; a run that needs the person waits for them, as a chat does, holds
//! no room under the task limit meanwhile, and stops when stopped, archived or deleted, or as
//! the server stops; a run waiting across a restart keeps its budget; a run due meanwhile is
//! skipped and recorded so once; a run refused otherwise is recorded with its reason, once for
//! the same; the scheduler never wakes at once for a run it would not start; deleting the chat
//! deletes the schedule; and a schedule is made on a copy of a repository or refused, its chat
//! gone when it is not kept. The chat says its schedule in its own facts, a list's watch hearing
//! it change, though never of what a first read of the ledger finds, nor in another
//! organization; and each run's turn says when it was due.

use glave_event::{At, Outcome, ThreadId, When};
use glave_ports::testing::{ManualClock, TempDir};
use glave_ports::{Caller, OrgId};
use glave_ports::{EventLog, InMemoryLog};
use glave_session::protocol::Notification;
use glave_session::protocol::{Answer, Reply, Request, RunStatus, ScheduleInfo, TaskStatus};
use glave_session::{Answering, ListWatch, Watched};
use jiff::Timestamp;
use serde_json::json;

use crate::schedule_pausing::pause;
use crate::support::*;

pub(crate) fn at(text: &str) -> At {
    At::from(text.parse::<Timestamp>().unwrap())
}

pub(crate) fn set(now: &ManualClock, text: &str) {
    now.set_millis(at(text).timestamp().as_millisecond());
}

/// A server answering `answers`, in Ask first so a write asks, with clients to answer, on a
/// repository of its own in `name`, its clock at 10:03.
pub(crate) async fn asking(
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
pub(crate) async fn schedule<L: EventLog + 'static>(
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

pub(crate) async fn listed<L: EventLog + 'static>(server: &Tested<L>) -> Vec<ScheduleInfo> {
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
pub(crate) fn writes_after_asking() -> Vec<glave_ports::Completion> {
    let write = json!({"path": "note.md", "content": "hi"});
    vec![
        costing(answer(None, Some(("write", write))), 10, 5, 1),
        costing(answer(Some("Done."), None), 10, 5, 1),
        costing(answer(Some("Done."), None), 10, 5, 1),
    ]
}

#[test]
fn a_one_off_s_run_still_waiting_for_the_person_a_day_on_keeps_its_project_listed() {
    block_on!(async {
        let log = Shared::default();
        let (server, now, _repo, _dir) =
            asking("runs-one-off-waits", writes_after_asking(), log.clone()).await;
        let once = Request::ScheduleCreate {
            project: Some(scope().project()),
            name: Some("Once".into()),
            when: When::At(at("2026-10-10T11:00:00Z")),
            zone: None,
            prompt: "check the build".into(),
            budget: 10_000,
        };
        let Ok(Reply::Schedule { schedule: made }) = server.request(once).await else {
            panic!("a schedule");
        };
        let chat = made.thread.unwrap();
        set(&now, "2026-10-10T11:00:00Z");
        assert_eq!(server.tick().await.unwrap(), vec![chat]);
        asked_in(&server, chat).await;
        // A day on, its run still waits for the person: its project stays listed, so whichever
        // server fires schedules next finds the run to follow.
        set(&now, "2026-10-11T12:00:00Z");
        server.tick().await.unwrap();
        let stream = glave_ports::Scope::of_organization(scope().org());
        let listed = log
            .read(
                &stream,
                glave_ports::stream::Reserved::Scheduled.id(),
                None,
                usize::MAX,
            )
            .await
            .unwrap();
        let kinds: Vec<&str> = listed.iter().map(|raw| raw.kind.as_str()).collect();
        assert_eq!(
            kinds,
            ["schedule.placed"],
            "never unplaced while its run waits"
        );
    });
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
        let (server, now, repo, dir) = asking("runs-refused", Vec::new(), Shared::default()).await;
        schedule(&server, "Daily check", "0 11 * * *", 10_000).await;
        // The project's folder, and the chat's copy of it, are gone.
        std::fs::remove_dir_all(&repo).unwrap();
        std::fs::remove_dir_all(dir.join("worktrees")).unwrap();
        set(&now, "2026-10-10T11:00:00Z");
        assert_eq!(server.tick().await.unwrap(), Vec::new());
        let last = listed(&server).await[0].last.clone().unwrap();
        assert_eq!(last.status, RunStatus::Refused);
        assert_eq!(
            last.said,
            "Couldn't start: its project's folder isn't there anymore"
        );
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
        let (server, now, repo, dir) =
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
        // Stopped, and its project's folder gone, the next is refused for that, another reason.
        let stopped = server.request(Request::TasksStop { thread: chat }).await;
        assert!(stopped.is_ok(), "{stopped:?}");
        settle(&server).await;
        std::fs::remove_dir_all(&repo).unwrap();
        std::fs::remove_dir_all(dir.join("worktrees")).unwrap();
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
pub(crate) async fn open_tasks<L: EventLog + 'static>(server: &Tested<L>) -> usize {
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
        log.fail_on("schedule.created");
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
        let person = server.person();
        server.shutdown().await;
        drop(server);
        let mut later = Setup::asking(writes_after_asking().split_off(1), Answering::Clients);
        later.clock = now.clone();
        later.settings.person = person;
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

#[test]
fn a_clean_copy_catches_up_with_the_project_before_a_run_and_one_with_changes_is_left_and_said() {
    block_on!(async {
        let answers = (0..3).map(|_| answer(Some("Done."), None)).collect();
        let mut setup = Setup::asking(answers, Answering::Clients);
        setup.clock =
            ManualClock::at_millis(at("2026-10-10T10:03:00Z").timestamp().as_millisecond());
        let now = setup.clock.clone();
        let (server, seen, repo, _dir) = setup
            .build_in_repository("runs-copy", InMemoryLog::new())
            .await;
        let chat = schedule(&server, "Build check", "0 11 * * *", 10_000)
            .await
            .thread
            .unwrap();
        let commit = |file: &str| {
            let script = format!(
                "echo new > {file} && git add . && \
                 git -c user.name=t -c user.email=t@example.com commit -q -m {file}"
            );
            sh(&repo, &script);
        };
        // The project moves on after the chat's copy was made.
        commit("news.md");
        set(&now, "2026-10-10T11:00:00Z");
        assert_eq!(server.tick().await.unwrap(), vec![chat]);
        settle(&server).await;
        let (_, root) = opened(&server, chat).await;
        let root = std::path::PathBuf::from(root);
        assert!(root.join("news.md").exists(), "brought up to date");
        // A copy with changes of its own is left as it is, and the run is told why, in a note
        // before the person's words, which stay as they were.
        std::fs::write(root.join("mine.md"), "mine").unwrap();
        commit("more.md");
        set(&now, "2026-10-11T11:00:00Z");
        assert_eq!(server.tick().await.unwrap(), vec![chat]);
        settle(&server).await;
        assert!(!root.join("more.md").exists(), "left as it was");
        let [note, words] = last_said(&seen);
        assert!(note.starts_with("<system-reminder>\n"), "{note}");
        assert!(
            note.contains("changes of its own not yet committed"),
            "{note}"
        );
        assert_eq!(words, "check the build");
        // Committed in the copy, while the project moved on too: it is told they diverged.
        sh(
            &root,
            "git add . && git -c user.name=t -c user.email=t@example.com commit -q -m mine",
        );
        set(&now, "2026-10-12T11:00:00Z");
        assert_eq!(server.tick().await.unwrap(), vec![chat]);
        settle(&server).await;
        let [note, _] = last_said(&seen);
        assert!(note.contains("has commits of its own"), "{note}");
    });
}

/// The texts of the last message sent to the model: the note before the person's words, and
/// their words.
fn last_said(seen: &Seen) -> [String; 2] {
    let requests = seen.lock().unwrap().clone();
    let last = requests.last().unwrap().messages.last().unwrap().clone();
    let glave_ports::Message::User { parts } = last else {
        panic!("the person's message, not {last:?}");
    };
    let texts: Vec<String> = (parts.into_iter())
        .filter_map(|part| match part {
            glave_event::Part::Text { text } => Some(text),
            _ => None,
        })
        .collect();
    texts.try_into().unwrap()
}

#[test]
fn a_schedule_s_chat_says_its_schedule_and_a_list_hears_it_change() {
    block_on!(async {
        let (server, _, _, _dir) = asking("runs-facts", Vec::new(), Shared::default()).await;
        let made = schedule(&server, "Daily check", "0 11 * * *", 10_000).await;
        let chat = made.thread.unwrap();
        let Reply::Thread { thread } = server
            .request(Request::ThreadGet { thread: chat })
            .await
            .unwrap()
        else {
            panic!("the chat");
        };
        let said = thread.schedule.unwrap();
        assert_eq!(said.schedule, made.schedule);
        assert_eq!(said.repeats, "Every day at 11:00 am");
        assert_eq!(said.next, Some(at("2026-10-10T11:00:00Z")));
        assert!(!said.paused);
        let mut watch = server
            .watch_list(&Caller::default(), None, None, false)
            .await
            .unwrap();
        pause(&server, made.schedule, true).await;
        loop {
            let Some(Watched::Note(note)) = watch.next().await else {
                panic!("the watch ended");
            };
            if let Notification::Changed { summary, .. } = note.as_ref()
                && summary.id == chat
                && summary.schedule.as_ref().is_some_and(|said| said.paused)
            {
                break;
            }
        }
    });
}

#[test]
fn each_run_s_turn_says_when_it_was_due() {
    block_on!(async {
        let answers = vec![answer(Some("Done."), None)];
        let (server, now, _, _dir) = asking("runs-due", answers, Shared::default()).await;
        let made = schedule(&server, "Daily check", "0 11 * * *", 10_000).await;
        let chat = made.thread.unwrap();
        set(&now, "2026-10-10T11:00:30Z");
        assert_eq!(server.tick().await.unwrap(), vec![chat]);
        settle(&server).await;
        let due: Vec<Option<At>> = (events(&server, chat).await.into_iter())
            .filter_map(|raw| match glave_event::decode(raw).unwrap().body {
                glave_event::Body::TurnStarted(started) => Some(started.scheduled),
                _ => None,
            })
            .collect();
        assert_eq!(due, vec![Some(at("2026-10-10T11:00:00Z"))]);
    });
}

/// Of `wanted`, the chat a list's `watch` is first told changed.
async fn first_heard(watch: &mut ListWatch, wanted: &[ThreadId]) -> ThreadId {
    loop {
        let Some(Watched::Note(note)) = watch.next().await else {
            panic!("the watch ended");
        };
        if let Notification::Changed { thread, .. } = note.as_ref()
            && wanted.contains(thread)
        {
            return *thread;
        }
    }
}

/// A chat `caller` starts in their Chats.
async fn chat_of<L: EventLog + 'static>(server: &Tested<L>, caller: &Caller) -> ThreadId {
    let started = Request::ThreadStart {
        project: None,
        worktree: None,
        background: false,
        computer: None,
        model: None,
        permissions: None,
        effort: None,
    };
    match server.request_as(caller, started).await.unwrap() {
        Reply::Thread { thread } => thread.id,
        other => panic!("a chat, not {other:?}"),
    }
}

#[test]
fn a_ledger_read_for_the_first_time_tells_lists_nothing_of_its_past() {
    block_on!(async {
        let log = Shared::default();
        let (server, now, repo, _dir) = asking("runs-first-read", Vec::new(), log.clone()).await;
        let made = schedule(&server, "Daily check", "0 11 * * *", 10_000).await;
        pause(&server, made.schedule, true).await;
        server.shutdown().await;
        drop(server);
        let mut later = Setup::asking(Vec::new(), Answering::Clients);
        later.clock = now.clone();
        later.worktrees = repo.parent().map(|dir| dir.join("worktrees"));
        let (restarted, _) = later.build_on(&repo, log).await;
        let here = Caller::default();
        let mut watch = restarted
            .watch_list(&here, None, None, false)
            .await
            .unwrap();
        // Its first read of the ledger finds the schedule made and paused before it started.
        restarted.tick().await.unwrap();
        let since = chat_of(&restarted, &here).await;
        let chat = made.thread.unwrap();
        assert_eq!(first_heard(&mut watch, &[chat, since]).await, since);
    });
}

#[test]
fn a_list_watched_in_another_organization_hears_nothing_of_a_schedule_here() {
    block_on!(async {
        let (server, _, _, _dir) = asking("runs-elsewhere", Vec::new(), Shared::default()).await;
        let third = OrgId::from_uuid(uuid::Uuid::from_u128(3));
        let grace = Caller {
            org: Some(third),
            ..elsewhere()
        };
        let mut watch = server.watch_list(&grace, None, None, false).await.unwrap();
        let made = schedule(&server, "Daily check", "0 11 * * *", 10_000).await;
        pause(&server, made.schedule, true).await;
        // What she hears first is a chat she starts in her own organization.
        let hers = chat_of(&server, &grace).await;
        let chat = made.thread.unwrap();
        assert_eq!(first_heard(&mut watch, &[chat, hers]).await, hers);
    });
}
