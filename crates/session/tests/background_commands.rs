//! A thread's commands in the background, through the API: listed while they run, stopped by
//! the person who asks (recorded as theirs), refused when there is no such command, and stopped
//! when the thread is archived.

use glave_event::{Body, CommandEnding, PrincipalKind, decode};
use glave_session::protocol::{Reply, Request, code};
use serde_json::json;

use crate::support::*;

fn starts_a_server() -> Vec<glave_ports::Completion> {
    vec![
        answer(
            None,
            Some(("bash", json!({"command": "sleep 30", "background": true}))),
        ),
        answer(Some("It is running."), None),
    ]
}

async fn listed<L: glave_ports::EventLog + 'static>(
    server: &Tested<L>,
    thread: glave_event::ThreadId,
) -> Vec<glave_harness::RunningCommand> {
    match server
        .request(Request::CommandList { thread })
        .await
        .unwrap()
    {
        Reply::Commands { commands } => commands,
        other => panic!("commands, not {other:?}"),
    }
}

#[test]
fn a_command_is_listed_while_it_runs_and_stopped_by_the_person() {
    let dir = folder("bg-api");
    block_on!(async {
        let (server, _) = Setup::new(starts_a_server()).build(&dir).await;
        let thread = start(&server, false).await;
        turn(&server, thread, "start the server").await.unwrap();
        let running = listed(&server, thread).await;
        assert_eq!(running.len(), 1, "{running:?}");
        assert_eq!(running[0].command, "sleep 30");
        assert!(running[0].started_at.is_some());
        assert!(running[0].summary.starts_with("Still running"));

        let stop = Request::CommandStop {
            thread,
            command: running[0].id,
        };
        assert!(matches!(
            server.request(stop.clone()).await,
            Ok(Reply::Done {})
        ));
        assert_eq!(listed(&server, thread).await, []);
        let again = server.request(stop).await.unwrap_err();
        assert_eq!(again.code, code::NO_COMMAND);

        let endings: Vec<CommandEnding> = events(&server, thread)
            .await
            .into_iter()
            .filter_map(|raw| match decode(raw).unwrap().body {
                Body::CommandEnded(ended) => Some(ended.ending),
                _ => None,
            })
            .collect();
        let [CommandEnding::Stopped { by: Some(by) }] = endings.as_slice() else {
            panic!("stopped by the person, not {endings:?}");
        };
        assert_eq!(by.kind, PrincipalKind::Person);
    });
}

#[test]
fn archiving_a_thread_stops_its_commands() {
    let dir = folder("bg-api-archived");
    block_on!(async {
        let (server, _) = Setup::new(starts_a_server()).build(&dir).await;
        let thread = start(&server, false).await;
        turn(&server, thread, "start the server").await.unwrap();
        server
            .request(Request::ThreadArchive { thread })
            .await
            .unwrap();
        let kinds = kinds(&server, thread).await;
        assert_eq!(
            kinds[kinds.len() - 2..],
            ["command.ended", "thread.archived"]
        );
        assert_eq!(listed(&server, thread).await, []);
    });
}
