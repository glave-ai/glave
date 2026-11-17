//! What bounds the agent's `schedule` tool: a schedule's own run, which is offered it so its
//! prefix stays the chat's, only lists, never setting up or removing what repeats; the hooks
//! before a tool run for every action of it; in someone else's chat it acts as them, never as
//! the person on the server's computer; and a schedule is made in the zone its card showed, a
//! zone the person never gave named on it.

use glave_event::{Body, HookPoint, HookResult, Permissions, When, decode};
use glave_ports::InMemoryLog;
use glave_ports::settings::{Hook, Hooks};
use glave_session::Sent;
use glave_session::protocol::{Answer, Reply, Request};
use serde_json::json;

use crate::schedule_tool::{asked, asked_of, create, decide, decide_as, schedules, setup};
use crate::support::*;

/// A schedule's id that no schedule has.
const SOMEONE_S: &str = "0192f5c2-7000-7000-8000-000000000001";

#[test]
fn a_schedule_s_own_run_lists_what_repeats_but_never_sets_up_or_removes_any() {
    block_on!(async {
        let remove = json!({"action": "remove", "schedule": SOMEONE_S});
        let calls = vec![create(), remove, json!({"action": "list"})];
        let ran = setup(Permissions::Auto, calls);
        let now = ran.clock.clone();
        let (server, seen, _, _dir) = ran
            .build_in_repository("tool-in-a-run", InMemoryLog::new())
            .await;
        let made = Request::ScheduleCreate {
            project: Some(scope().project()),
            name: Some("Morning check".into()),
            when: When::Cron("0 11 * * *".into()),
            zone: Some("UTC".into()),
            prompt: "check the build".into(),
            budget: 10_000,
        };
        let Reply::Schedule { schedule } = server.request(made).await.unwrap() else {
            panic!("a schedule");
        };
        let chat = schedule.thread.unwrap();
        // Monday 16 November 2026, 11:00 UTC: its run is due.
        now.set_millis(1_794_826_800_000);
        assert_eq!(server.tick().await.unwrap(), vec![chat]);
        settle(&server).await;
        let told = json(&seen);
        assert!(
            told[0].contains("\"name\":\"schedule\""),
            "offered, as in its chat"
        );
        for refused in [&told[1], &told[2]] {
            assert!(
                refused.contains("Not done: this is a run of repeating work"),
                "{refused}"
            );
        }
        assert!(told[3].contains("Morning check"), "it lists: {}", told[3]);
        assert_eq!(asked(&server, chat).await, None, "no one was asked");
        assert_eq!(schedules(&server).await.len(), 1, "nothing made or removed");
    });
}

#[test]
fn the_hooks_before_a_tool_run_for_every_action_of_it() {
    block_on!(async {
        let remove = json!({"action": "remove", "schedule": SOMEONE_S});
        let calls = vec![create(), json!({"action": "list"}), remove];
        let mut hooked = setup(Permissions::Auto, calls);
        let hook = Hook {
            point: HookPoint::PreToolUse,
            command: "echo 'not today' >&2; exit 2".into(),
            matcher: Some("schedule".into()),
            timeout_secs: None,
        };
        hooked.hooks = Hooks::new(vec![hook]).unwrap();
        let (server, seen, _, _dir) = hooked
            .build_in_repository("tool-hooked", InMemoryLog::new())
            .await;
        let thread = start(&server, false).await;
        turn(&server, thread, "check the build every weekday at 8")
            .await
            .unwrap();
        let told = json(&seen);
        for stopped in told.iter().skip(1) {
            assert!(
                stopped.contains("Not run: a hook the person set up stopped this call: not today"),
                "{stopped}"
            );
        }
        assert_eq!(told.len(), 4, "create, list, remove, then done");
        let blocked = (events(&server, thread).await.into_iter())
            .filter_map(|raw| match decode(raw).ok()?.body {
                Body::HookRan(ran) => Some((ran.point, ran.result)),
                _ => None,
            })
            .collect::<Vec<_>>();
        let want = (HookPoint::PreToolUse, HookResult::Blocked);
        assert_eq!(blocked, vec![want, want, want], "create, list and remove");
        assert_eq!(asked(&server, thread).await, None, "no one was asked");
        assert_eq!(schedules(&server).await, Vec::new());
    });
}

#[test]
fn in_someone_else_s_chat_the_tool_acts_as_them_never_as_the_person_here() {
    block_on!(async {
        let (server, _, _, _dir) = setup(Permissions::Auto, vec![create()])
            .build_in_repository("tool-theirs", InMemoryLog::new())
            .await;
        let grace = elsewhere();
        let start = Request::ThreadStart {
            project: None,
            background: false,
            worktree: None,
            computer: None,
            model: None,
            permissions: None,
            effort: None,
        };
        let Reply::Thread { thread } = server.request_as(&grace, start).await.unwrap() else {
            panic!("a chat of hers");
        };
        let said = turn_start(thread.id, "check the build every weekday at 8", None);
        let Sent::Started(_turn) = server.start_streaming(&grace, said).await.unwrap() else {
            panic!("a turn of its own");
        };
        asked_in_as(&server, &grace, thread.id).await;
        let card = asked_of(&server, &grace, thread.id).await.unwrap();
        assert!(
            card.detail.starts_with("Monday to Friday, 8:00 am UTC."),
            "she never gave a zone, so it is named: {}",
            card.detail
        );
        decide_as(&server, &grace, thread.id, Answer::AllowOnce).await;
        let hers = server.request_as(&grace, Request::ScheduleList {}).await;
        let Ok(Reply::Schedules { schedules: hers }) = hers else {
            panic!("her schedules: {hers:?}");
        };
        assert_eq!(hers.len(), 1, "made as her");
        assert_eq!(hers[0].from, Some(thread.id));
        assert_ne!(hers[0].project, scope().project(), "in her own Chats");
        assert_eq!(
            schedules(&server).await,
            Vec::new(),
            "none of the person's here"
        );
    });
}

#[test]
fn a_schedule_is_made_in_the_zone_its_card_showed() {
    block_on!(async {
        let (server, _, _, _dir) = setup(Permissions::Auto, vec![create()])
            .build_in_repository("tool-zone", InMemoryLog::new())
            .await;
        let thread = start(&server, false).await;
        let _turn = stream_turn(&server, thread, "check the build every weekday at 8", None)
            .await
            .unwrap();
        asked_in(&server, thread).await;
        let card = asked(&server, thread).await.unwrap();
        assert!(
            card.detail
                .starts_with("Monday to Friday, 8:00 am UTC. First run: Tuesday 17 November."),
            "{}",
            card.detail
        );
        // Their browser says where they are while the card waits.
        let zone = Request::ZoneSet {
            zone: "America/New_York".into(),
        };
        server.request(zone).await.unwrap();
        decide(&server, thread, Answer::AllowOnce).await;
        let made = schedules(&server).await;
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].zone, "UTC", "as the card showed, not as set since");
    });
}
