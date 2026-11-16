//! The agent's `schedule` tool through the session API: it is offered in a chat someone can
//! answer in; setting one up asks the person with the server's own sentence, in every mode,
//! Full access included, and only their yes makes the schedule, with its chat, in their time
//! zone, remembering the chat it was asked in; "Not now" makes nothing; and the agent lists and
//! removes schedules when asked.

use glave_event::{Ask, Body, Permissions, ThreadId, decode};
use glave_ports::testing::ManualClock;
use glave_ports::{EventLog, InMemoryLog};
use glave_session::Answering;
use glave_session::protocol::{Answer, Reply, Request, ScheduleInfo};
use serde_json::json;

use crate::support::*;

/// What the model calls to set up a schedule every weekday at 8.
fn create() -> serde_json::Value {
    json!({
        "action": "create", "name": "Morning check", "prompt": "check the build",
        "cron": "0 8 * * 1-5", "budget": 10000
    })
}

/// A server in `mode` on a repository of its own, whose model makes `call` then says it is done;
/// what its model was sent; and the folder.
async fn server(
    name: &str,
    mode: Permissions,
    calls: Vec<serde_json::Value>,
) -> (Tested, Seen, glave_ports::testing::TempDir) {
    let mut answers: Vec<_> = (calls.into_iter())
        .map(|call| answer(None, Some(("schedule", call))))
        .collect();
    answers.push(answer(Some("Done."), None));
    let mut setup = Setup::asking(answers, Answering::Clients);
    setup.settings.permissions = mode;
    setup.clock = ManualClock::at_millis(1_791_000_000_000);
    let (server, seen, _, dir) = setup.build_in_repository(name, InMemoryLog::new()).await;
    (server, seen, dir)
}

async fn schedules<L: EventLog + 'static>(server: &Tested<L>) -> Vec<ScheduleInfo> {
    match server.request(Request::ScheduleList {}).await.unwrap() {
        Reply::Schedules { schedules } => schedules,
        other => panic!("schedules, not {other:?}"),
    }
}

/// Lets `thread`'s turn run until it has ended.
async fn ended<L: EventLog + 'static>(server: &Tested<L>, thread: ThreadId) {
    for _ in 0..100_000 {
        if kinds(server, thread)
            .await
            .iter()
            .any(|kind| kind == "turn.ended")
        {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("the turn never ended");
}

/// The person answers `thread`'s waiting approval with `decision`, and the turn goes on to
/// its end.
async fn decide<L: EventLog + 'static>(server: &Tested<L>, thread: ThreadId, decision: Answer) {
    let approval = asked_in(server, thread).await;
    let decided = Request::ApprovalDecide {
        thread,
        approval,
        decision,
    };
    let answered = server.request(decided).await;
    assert!(answered.is_ok(), "{answered:?}");
    ended(server, thread).await;
}

#[test]
fn the_agent_sets_up_a_schedule_only_once_the_person_says_yes_to_the_server_s_sentence() {
    block_on!(async {
        let (server, seen, _dir) = server("tool-yes", Permissions::Auto, vec![create()]).await;
        let zone = Request::ZoneSet {
            zone: "America/New_York".into(),
        };
        server.request(zone).await.unwrap();
        let thread = start(&server, false).await;
        let _turn = stream_turn(&server, thread, "check the build every weekday at 8", None)
            .await
            .unwrap();
        asked_in(&server, thread).await;
        let sent = json(&seen);
        assert!(
            sent[0].contains("\"name\":\"schedule\""),
            "offered in a chat"
        );
        let asked = (events(&server, thread).await.into_iter())
            .find_map(|raw| match decode(raw).ok()?.body {
                Body::ApprovalRequested(asked) => Some(asked),
                _ => None,
            })
            .unwrap();
        assert_eq!(asked.ask, "Repeat this every weekday at 8:00 am?");
        assert_eq!(
            asked.asking,
            Some(Ask::Repeat {
                when: "every weekday at 8:00 am".into()
            })
        );
        assert_eq!(asked.detail, "check the build", "what each run does");
        assert_eq!(
            schedules(&server).await,
            Vec::new(),
            "nothing before the yes"
        );
        decide(&server, thread, Answer::AllowOnce).await;
        let made = schedules(&server).await;
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].name, "Morning check");
        assert_eq!(
            made[0].from,
            Some(thread),
            "it remembers the chat it came from"
        );
        assert_eq!(made[0].zone, "America/New_York");
        assert_ne!(made[0].thread, Some(thread), "a chat of its own");
        let told = json(&seen);
        assert!(
            told[1].contains("Set up \\\"Morning check\\\""),
            "{}",
            told[1]
        );
    });
}

#[test]
fn not_now_sets_up_nothing_and_full_access_still_asks() {
    block_on!(async {
        let (server, seen, _dir) = server("tool-no", Permissions::FullAccess, vec![create()]).await;
        let thread = start(&server, false).await;
        let _turn = stream_turn(&server, thread, "check the build every weekday", None)
            .await
            .unwrap();
        decide(&server, thread, Answer::Deny).await;
        assert_eq!(schedules(&server).await, Vec::new());
        let told = json(&seen);
        assert!(told[1].contains("The person declined"), "{}", told[1]);
    });
}

#[test]
fn the_agent_lists_the_person_s_schedules_and_removes_only_one_there_is() {
    block_on!(async {
        let calls = vec![
            json!({"action": "list"}),
            json!({"action": "remove", "schedule": "0192f5c1-0000-7000-8000-00000000dead"}),
        ];
        let (server, seen, _dir) = server("tool-list", Permissions::Auto, calls).await;
        let made = Request::ScheduleCreate {
            project: Some(scope().project()),
            name: Some("Evening tidy".into()),
            when: glave_event::When::Cron("0 19 * * *".into()),
            zone: Some("Europe/Lisbon".into()),
            prompt: "tidy the notes".into(),
            budget: 1_000,
        };
        let Reply::Schedule { schedule } = server.request(made).await.unwrap() else {
            panic!("a schedule");
        };
        let thread = start(&server, false).await;
        turn(&server, thread, "what repeats?").await.unwrap();
        let told = json(&seen);
        assert!(told[1].contains("Evening tidy"), "{}", told[1]);
        assert!(
            told[1].contains("every day at 7:00 pm, Europe/Lisbon time"),
            "{}",
            told[1]
        );
        assert!(
            told[1].contains(&schedule.schedule.to_string()),
            "{}",
            told[1]
        );
        assert!(told[2].contains("no schedule"), "{}", told[2]);
        assert_eq!(schedules(&server).await.len(), 1, "nothing removed");
    });
}
