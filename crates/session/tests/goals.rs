//! Goals through the session API against a scripted model: `goal/start` streams every turn of
//! a goal until the model says it is met, over JSON-RPC as in-process; `goal/stop` stops a
//! running goal; `goal/get` shows progress from the log; and the errors when there is no goal.

use std::path::Path;
use std::sync::Arc;

use glave_event::{GoalReason, ThreadId};
use glave_harness::GoalProgress;
use glave_ports::{Caller, Completion};
use glave_session::protocol::{Reply, Request, code};
use glave_session::{Answering, TurnItem};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::sync::Notify;

use crate::support::*;

/// A server in auto mode, where the check runs without asking.
async fn auto(dir: &Path, answers: Vec<Completion>, gate: Option<Arc<Notify>>) -> Tested {
    let mut setup = Setup::new(answers);
    setup.gate = gate;
    setup.answering = Answering::NoOne;
    setup.build(dir).await.0
}

async fn goal(server: &Tested, thread: ThreadId) -> Option<GoalProgress> {
    match server.request(Request::GoalGet { thread }).await.unwrap() {
        Reply::Goal { goal } => goal,
        other => panic!("expected a goal, got {other:?}"),
    }
}

#[test]
fn goal_start_streams_every_turn_until_the_model_says_it_is_met_over_json_rpc() {
    let dir = folder("goal-rpc");
    block_on!(async {
        let answers = vec![
            answer(Some("Looking."), None),
            answer(
                None,
                Some(("write", json!({"path": "ok.txt", "content": "ok"}))),
            ),
            answer(None, Some(("update_goal", json!({"status": "complete"})))),
            answer(Some("Done."), None),
        ];
        let server = auto(&dir, answers, None).await;
        let thread = start(&server, false).await;
        let (client_out, server_in) = tokio::io::duplex(1 << 16);
        let (server_out, client_in) = tokio::io::duplex(1 << 16);
        let serving = tokio::spawn(glave_session::rpc::serve(
            server.clone(),
            BufReader::new(server_in),
            server_out,
        ));
        let mut lines = BufReader::new(client_in).lines();
        let mut to_server = client_out;
        let request = json!({"jsonrpc": "2.0", "id": 1, "method": "goal/start", "params": {
            "thread": thread, "objective": "make ok.txt; check it exists"
        }});
        to_server
            .write_all(format!("{request}\n").as_bytes())
            .await
            .unwrap();
        let mut types = Vec::new();
        let result = loop {
            let line = lines.next_line().await.unwrap().unwrap();
            let message: Value = serde_json::from_str(&line).unwrap();
            if message["id"] == 1 {
                break message.clone();
            }
            if message["method"] == "thread/event" {
                types.push(
                    message["params"]["event"]["type"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                );
            }
        };
        assert_eq!(
            result["result"]["turn"]["model_calls"], 4,
            "{result} {types:?}"
        );
        assert_eq!(types.first().map(String::as_str), Some("goal.set"));
        assert_eq!(types.last().map(String::as_str), Some("goal.ended"));
        assert_eq!(
            types.iter().filter(|kind| *kind == "turn.started").count(),
            2
        );
        to_server.shutdown().await.unwrap();
        drop(to_server);
        serving.await.unwrap().unwrap();
        let progress = goal(&server, thread).await.unwrap();
        assert_eq!(progress.ended, Some(GoalReason::Met));
        assert_eq!(progress.turns, 2);
    });
}

#[test]
fn goal_stop_stops_a_running_goal() {
    let dir = folder("goal-stop");
    block_on!(async {
        let gate = Arc::new(Notify::new());
        let answers = vec![answer(Some("Thinking."), None)];
        let server = auto(&dir, answers, Some(Arc::clone(&gate))).await;
        let thread = start(&server, false).await;
        let goal_of = glave_harness::Goal::new("forever");
        let mut stream = server
            .start_goal_as(&Caller::default(), thread, Some(goal_of))
            .await
            .unwrap();
        // Its first event is `goal.set`.
        assert!(matches!(stream.next().await, Some(TurnItem::Note(_))));
        // Running: the goal is active, and progress is readable from the log.
        let progress = goal(&server, thread).await.unwrap();
        assert!(progress.active());
        assert_eq!(progress.objective, "forever");
        let stop = server.request(Request::GoalStop { thread }).await.unwrap();
        assert!(matches!(stop, Reply::Done {}));
        let mut ended = None;
        while let Some(item) = stream.next().await {
            if let TurnItem::Ended(result) = item {
                ended = Some(result);
            }
        }
        assert!(ended.unwrap().is_ok());
        assert_eq!(
            goal(&server, thread).await.unwrap().ended,
            Some(GoalReason::Stopped)
        );
    });
}

#[test]
fn without_a_goal_stop_and_continue_are_refused() {
    let dir = folder("goal-none");
    block_on!(async {
        let server = auto(&dir, vec![], None).await;
        let thread = start(&server, false).await;
        assert_eq!(goal(&server, thread).await, None);
        let stop = server.request(Request::GoalStop { thread }).await;
        assert_eq!(stop.unwrap_err().code, code::NO_GOAL);
        let resume = Request::GoalStart {
            background: false,
            thread,
            objective: None,
            budget: None,
        };
        assert_eq!(
            server.request(resume).await.unwrap_err().code,
            code::NO_GOAL
        );
        let unknown =
            ThreadId::new(jiff::Timestamp::from_millisecond(5).unwrap(), [9; 10]).unwrap();
        let get = server.request(Request::GoalGet { thread: unknown }).await;
        assert_eq!(get.unwrap_err().code, code::NO_THREAD);
    });
}
