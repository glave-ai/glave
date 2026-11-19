//! Pausing and resuming a schedule: paused, it starts no runs, and resumed its runs count from
//! then; a one-off whose time came while it was paused is missed, and one due before is not;
//! archiving its chat pauses it, and it is not resumed while the chat is archived, nor one made
//! before schedules had chats; and a paused schedule still counts toward the most a person
//! keeps.

use glave_event::{RawEvent, When};
use glave_ports::EventLog;
use glave_ports::stream::Reserved;
use glave_session::protocol::{Reply, Request, RunStatus, ScheduleInfo, code};

use crate::schedule_runs::{asking, at, listed, schedule, set};
use crate::support::*;

/// `schedule/pause` of `schedule`, pausing it (`paused`) or resuming it, answered.
pub(crate) async fn pause<L: EventLog + 'static>(
    server: &Tested<L>,
    schedule: glave_event::ScheduleId,
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
        let answers = (0..2).map(|_| answer(Some("Done."), None)).collect();
        let (server, now, _, _dir) = asking("runs-pause", answers, Shared::default()).await;
        let made = schedule(&server, "Hourly", "0 * * * *", 10_000).await;
        let paused = pause(&server, made.schedule, true).await;
        assert!(paused.paused);
        assert_eq!((paused.next, server.next_due()), (None, None));
        assert!(pause(&server, made.schedule, true).await.paused, "as it is");
        set(&now, "2026-10-10T13:30:00Z");
        assert_eq!(
            server.tick().await.unwrap(),
            Vec::new(),
            "paused through three runs"
        );
        let resumed = pause(&server, made.schedule, false).await;
        assert!(!resumed.paused);
        assert_eq!(resumed.next, Some(at("2026-10-10T14:00:00Z")));
        assert_eq!(
            server.tick().await.unwrap(),
            Vec::new(),
            "none for the paused time"
        );
        set(&now, "2026-10-10T14:00:00Z");
        assert_eq!(server.tick().await.unwrap().len(), 1);
    });
}

/// A schedule of `server`'s that runs once, at 11:00.
async fn once_at_11(server: &Tested<Shared>) -> ScheduleInfo {
    let request = Request::ScheduleCreate {
        project: Some(scope().project()),
        name: Some("Once".into()),
        when: When::At(at("2026-10-10T11:00:00Z")),
        zone: None,
        prompt: "check the build".into(),
        budget: 10_000,
    };
    match server.request(request).await.unwrap() {
        Reply::Schedule { schedule } => schedule,
        other => panic!("a schedule, not {other:?}"),
    }
}

#[test]
fn a_one_off_whose_time_passed_while_paused_says_it_was_missed() {
    block_on!(async {
        let (server, now, _, _dir) = asking("runs-missed", Vec::new(), Shared::default()).await;
        let schedule = once_at_11(&server).await;
        pause(&server, schedule.schedule, true).await;
        set(&now, "2026-10-10T11:30:00Z");
        let resumed = pause(&server, schedule.schedule, false).await;
        assert_eq!(resumed.next, None, "it runs no more");
        let last = resumed.last.unwrap();
        assert_eq!(
            (last.status, last.due),
            (RunStatus::Missed, at("2026-10-10T11:00:00Z"))
        );
        assert_eq!(last.said, "Missed: it was paused at its time");
        assert_eq!(server.tick().await.unwrap(), Vec::new());
    });
}

#[test]
fn archiving_a_schedule_s_chat_pauses_the_schedule() {
    block_on!(async {
        let (server, _, _, _dir) = asking("runs-archived", Vec::new(), Shared::default()).await;
        let chat = schedule(&server, "Daily", "0 11 * * *", 10_000)
            .await
            .thread
            .unwrap();
        server
            .request(Request::ThreadArchive { thread: chat })
            .await
            .unwrap();
        assert!(listed(&server).await[0].paused);
    });
}

#[test]
fn a_one_off_due_before_it_was_paused_is_not_missed() {
    block_on!(async {
        let answers = vec![answer(Some("Done."), None)];
        let (server, now, _, _dir) = asking("runs-not-missed", answers, Shared::default()).await;
        let schedule = once_at_11(&server).await;
        // Its time comes while no server ticks, then it is paused and resumed.
        set(&now, "2026-10-10T11:30:00Z");
        pause(&server, schedule.schedule, true).await;
        set(&now, "2026-10-10T11:45:00Z");
        let resumed = pause(&server, schedule.schedule, false).await;
        assert_eq!(
            resumed.last, None,
            "not missed: it came due before the pause"
        );
        assert_eq!(server.tick().await.unwrap().len(), 1, "it runs, late, once");
        settle(&server).await;
    });
}

#[test]
fn pausing_makes_no_room_under_the_most_schedules_kept() {
    block_on!(async {
        let mut setup = Setup::asking(Vec::new(), glave_session::Answering::Clients);
        setup.limits.max_schedules = Some(1);
        let (server, _, _, _dir) = setup
            .build_in_repository("runs-pause-limit", Shared::default())
            .await;
        let made = schedule(&server, "Daily", "0 11 * * *", 10_000).await;
        pause(&server, made.schedule, true).await;
        let another = Request::ScheduleCreate {
            project: Some(scope().project()),
            name: Some("Another".into()),
            when: When::Cron("0 12 * * *".into()),
            zone: None,
            prompt: "check the build".into(),
            budget: 10_000,
        };
        let refused = server.request(another).await.unwrap_err();
        assert_eq!(refused.code, code::LIMIT, "the paused one still counts");
    });
}

#[test]
fn a_schedule_is_not_resumed_while_its_chat_is_archived() {
    block_on!(async {
        let (server, _, _, _dir) =
            asking("runs-resume-archived", Vec::new(), Shared::default()).await;
        let made = schedule(&server, "Daily", "0 11 * * *", 10_000).await;
        let chat = made.thread.unwrap();
        server
            .request(Request::ThreadArchive { thread: chat })
            .await
            .unwrap();
        let resume = Request::SchedulePause {
            schedule: made.schedule,
            paused: false,
        };
        let refused = server.request(resume).await.unwrap_err();
        assert_eq!(refused.code, code::ARCHIVED);
        assert_eq!(
            refused.message.to_string(),
            "this schedule's chat is archived: unarchive the chat first, then resume the schedule"
        );
        assert!(listed(&server).await[0].paused, "still paused");
    });
}

#[test]
fn a_schedule_made_before_schedules_had_chats_is_not_resumed() {
    block_on!(async {
        let log = Shared::default();
        let (server, _, _, _dir) = asking("runs-resume-old", Vec::new(), log.clone()).await;
        // A schedule as version 1 recorded it, with no chat of its own.
        let at_ = scope();
        let ledger = log
            .read(&at_, Reserved::Tasks.id(), None, usize::MAX)
            .await
            .unwrap();
        let seq = ledger.last().map_or(1, |event| event.seq.get() + 1);
        let id = "0192f5c2-7000-7000-8000-0000000000aa";
        let schedule = "0192f5c1-0000-7000-8000-0000000000bb";
        let stored = format!(
            r#"{{"glave":1,"id":"{id}","thread":"{}","seq":{seq},"at":"2026-10-10T09:00:00.000Z","type":"schedule.created","v":1,"data":{{"schedule":"{schedule}","when":{{"cron":"0 9 * * *"}},"prompt":"Check the release","budget":50000}}}}"#,
            Reserved::Tasks.id()
        );
        log.append(&at_, &[RawEvent::from_json(&stored).unwrap()])
            .await
            .unwrap();
        let old = listed(&server).await[0].clone();
        assert!(old.paused);
        let resume = Request::SchedulePause {
            schedule: old.schedule,
            paused: false,
        };
        let refused = server.request(resume).await.unwrap_err();
        assert_eq!(refused.code, code::INVALID_PARAMS);
        assert!(listed(&server).await[0].paused, "still paused");
    });
}
