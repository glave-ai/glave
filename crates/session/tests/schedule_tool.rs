//! The agent's `schedule` tool through the session API: it is offered in a chat someone can
//! answer in, where schedules run, and never in a task; setting one up asks the person with the
//! server's own card (its question, its first run on their clock, its chat), in every mode, Full
//! access included, refused before asking for whatever would refuse it, and refused in plan
//! mode; only their yes makes the schedule, under the server's own budget, in their time zone,
//! remembering the chat it was asked in; "Not now" makes nothing; and the agent lists and
//! removes schedules when asked, removing one asking as any change does.

use glave_event::{Ask, Body, Permissions, ThreadId, decode};
use glave_ports::testing::ManualClock;
use glave_ports::{EventLog, InMemoryLog};
use glave_session::protocol::{Answer, Reply, Request, ScheduleInfo};
use glave_session::{Answering, KeepAwake, Unawake};
use serde_json::json;

use crate::support::*;

/// What the model calls to set up a schedule every weekday at 8.
fn create() -> serde_json::Value {
    json!({
        "action": "create", "name": "Morning check", "prompt": "check the build",
        "cron": "0 8 * * 1-5"
    })
}

/// A setup in `mode`, whose model makes each of `calls` then says it is done, on a clock at
/// Monday 16 November 2026, 10:00 UTC.
fn setup(mode: Permissions, calls: Vec<serde_json::Value>) -> Setup {
    let mut answers: Vec<_> = (calls.into_iter())
        .map(|call| answer(None, Some(("schedule", call))))
        .collect();
    answers.push(answer(Some("Done."), None));
    let mut setup = Setup::asking(answers, Answering::Clients);
    setup.settings.permissions = mode;
    setup.clock = ManualClock::at_millis(1_794_823_200_000);
    setup
}

/// A server in `mode` on a repository of its own, whose model makes each of `calls` then says
/// it is done; what its model was sent; and the folder.
async fn server(
    name: &str,
    mode: Permissions,
    calls: Vec<serde_json::Value>,
) -> (Tested, Seen, glave_ports::testing::TempDir) {
    let setup = setup(mode, calls);
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

/// What `thread` asked the person, recorded.
async fn asked<L: EventLog + 'static>(
    server: &Tested<L>,
    thread: ThreadId,
) -> Option<glave_event::ApprovalRequested> {
    (events(server, thread).await.into_iter()).find_map(|raw| match decode(raw).ok()?.body {
        Body::ApprovalRequested(asked) => Some(asked),
        _ => None,
    })
}

#[test]
fn the_agent_sets_up_a_schedule_only_once_the_person_says_yes_to_the_server_s_card() {
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
        assert!(
            !sent[0].contains("\"budget\""),
            "the server's budget, not the model's"
        );
        let asked = asked(&server, thread).await.unwrap();
        assert_eq!(asked.ask, "Repeat this every weekday at 8:00 am?");
        assert_eq!(
            asked.asking,
            Some(Ask::Repeat {
                when: "every weekday at 8:00 am".into()
            })
        );
        let repeating = asked.repeating.clone().unwrap();
        assert_eq!(
            (
                repeating.cron.as_str(),
                repeating.zone.as_str(),
                repeating.name.as_str()
            ),
            ("0 8 * * 1-5", "America/New_York", "Morning check"),
            "what the card was written from, to write it again in another language"
        );
        assert_eq!(
            asked.detail,
            "First run: Monday 16 November at 8:00 am, your time.\n\
             Each run is added to its own chat, \u{201c}Morning check\u{201d}."
        );
        assert_eq!(asked.caution, None, "no laptop caveat on a server");
        assert_eq!(
            schedules(&server).await,
            Vec::new(),
            "nothing before the yes"
        );
        decide(&server, thread, Answer::AllowOnce).await;
        let made = schedules(&server).await;
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].name, "Morning check");
        assert_eq!(made[0].from, Some(thread), "it remembers the chat");
        assert_eq!(made[0].zone, "America/New_York");
        assert_eq!(made[0].budget, 200_000, "the server's own budget");
        assert_ne!(made[0].thread, Some(thread), "a chat of its own");
        let result = (events(&server, thread).await.into_iter())
            .find_map(|raw| match decode(raw).ok()?.body {
                Body::ToolCallCompleted(done) => done.structured,
                _ => None,
            })
            .unwrap();
        assert_eq!(
            result["schedule"],
            json!(made[0].schedule),
            "the client finds the schedule"
        );
        assert_eq!(result["chat"], json!(made[0].thread));
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
fn what_would_refuse_a_schedule_refuses_it_before_anyone_is_asked() {
    block_on!(async {
        let blank = json!({"action": "create", "name": "x", "prompt": "  ", "cron": "0 8 * * *"});
        let mut full = setup(Permissions::Auto, vec![blank, create()]);
        full.limits.max_schedules = Some(0);
        let (server, seen, _, _dir) = full
            .build_in_repository("tool-refused", InMemoryLog::new())
            .await;
        let thread = start(&server, false).await;
        turn(&server, thread, "check the build every weekday")
            .await
            .unwrap();
        assert_eq!(asked(&server, thread).await, None, "no one was asked");
        let told = json(&seen);
        assert!(told[1].contains("a schedule needs a prompt"), "{}", told[1]);
        assert!(told[2].contains("max_schedules"), "{}", told[2]);
    });
}

#[test]
fn plan_mode_refuses_a_schedule() {
    block_on!(async {
        let (server, seen, _dir) = server("tool-plan", Permissions::Plan, vec![create()]).await;
        let thread = start(&server, false).await;
        turn(&server, thread, "check the build every weekday")
            .await
            .unwrap();
        assert_eq!(asked(&server, thread).await, None);
        let told = json(&seen);
        assert!(told[1].contains("Not set up in plan mode"), "{}", told[1]);
        assert_eq!(schedules(&server).await, Vec::new());
    });
}

#[test]
fn a_task_is_never_offered_a_schedule() {
    block_on!(async {
        let (tasked, seen, _dir) = server("tool-task", Permissions::Auto, Vec::new()).await;
        let thread = start(&tasked, true).await;
        background(&tasked, thread, Some(10_000)).await;
        settle(&tasked).await;
        let sent = json(&seen);
        assert!(
            !sent[0].contains("\"name\":\"schedule\""),
            "no one to confirm it"
        );
    });
}

#[test]
fn schedules_are_not_offered_where_none_fires_them() {
    block_on!(async {
        let mut none = setup(Permissions::Auto, Vec::new());
        none.schedules = false;
        let (server, seen, _, _dir) = none
            .build_in_repository("tool-none", InMemoryLog::new())
            .await;
        let thread = start(&server, false).await;
        turn(&server, thread, "hi").await.unwrap();
        assert!(!json(&seen)[0].contains("\"name\":\"schedule\""));
    });
}

#[test]
fn the_agent_lists_the_person_s_schedules_and_removing_one_asks_as_a_change_does() {
    block_on!(async {
        let made = |name: &str| Request::ScheduleCreate {
            project: Some(scope().project()),
            name: Some(name.into()),
            when: glave_event::When::Cron("0 19 * * *".into()),
            zone: Some("Europe/Lisbon".into()),
            prompt: "tidy the notes".into(),
            budget: 1_000,
        };
        let calls = |schedule: String| {
            vec![
                json!({"action": "list"}),
                json!({"action": "remove", "schedule": "0192f5c1-0000-7000-8000-00000000dead"}),
                json!({"action": "remove", "schedule": schedule}),
            ]
        };
        // Its id is known only once it is made: a server set up the same way, with the same
        // clock and randomness, makes the same one.
        let made_by = async |name: &str, calls| {
            let built = setup(Permissions::Ask, calls)
                .build_in_repository(name, InMemoryLog::new())
                .await;
            let Reply::Schedule { schedule } = built.0.request(made("Evening tidy")).await.unwrap()
            else {
                panic!("a schedule");
            };
            (built, schedule.schedule.to_string())
        };
        let (_probe, id) = made_by("tool-list-probe", Vec::new()).await;
        let ((server, seen, _, _dir), again) = made_by("tool-list", calls(id.clone())).await;
        assert_eq!(again, id, "the same schedule");
        let thread = start(&server, false).await;
        let _turn = stream_turn(&server, thread, "stop the evening tidy", None)
            .await
            .unwrap();
        asked_in(&server, thread).await;
        let told = json(&seen);
        assert!(told[1].contains("Evening tidy"), "{}", told[1]);
        assert!(
            told[1].contains("every day at 7:00 pm, Europe/Lisbon time"),
            "{}",
            told[1]
        );
        assert!(told[1].contains(&id), "{}", told[1]);
        assert!(told[2].contains("no schedule"), "{}", told[2]);
        let asked = asked(&server, thread).await.unwrap();
        assert_eq!(asked.ask, "Stop repeating Evening tidy?");
        assert_eq!(schedules(&server).await.len(), 1, "not before the yes");
        decide(&server, thread, Answer::AllowOnce).await;
        assert_eq!(schedules(&server).await, Vec::new());
    });
}

/// A person's own computer, which glave keeps awake while it works.
struct Laptop;

impl KeepAwake for Laptop {
    fn hold(&self) -> glave_ports::BoxFuture<'_, Option<Box<dyn Send>>> {
        Box::pin(async { None })
    }

    fn cannot(&self) -> Option<Unawake> {
        None
    }
}

#[test]
fn on_a_person_s_own_computer_the_card_says_it_runs_only_while_glave_runs() {
    block_on!(async {
        let mut laptop = setup(Permissions::Auto, vec![create()]);
        laptop.awake = Some(std::sync::Arc::new(Laptop));
        let (server, _, _, _dir) = laptop
            .build_in_repository("tool-laptop", InMemoryLog::new())
            .await;
        let thread = start(&server, false).await;
        let _turn = stream_turn(&server, thread, "check the build every weekday at 8", None)
            .await
            .unwrap();
        asked_in(&server, thread).await;
        let asked = asked(&server, thread).await.unwrap();
        assert_eq!(
            asked.caution.as_deref(),
            Some("It runs only while glave runs on this computer.")
        );
    });
}
