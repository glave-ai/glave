//! A thread's commands in the background, through the API: listed while they run, stopped by
//! the person who asks (recorded as theirs), refused when there is no such command, stopped
//! when the thread is archived, and recorded as ended as soon as another command kills one.

use std::time::Duration;

use glave_event::{Body, CommandEnding, PrincipalKind, decode};
use glave_ports::Caller;
use glave_session::Watched;
use glave_session::protocol::{Notification, Reply, Request, code};
use serde_json::json;

use crate::support::*;

/// A command that never ends by itself, on macOS and Linux alike, so only a stop ends it,
/// however long a busy machine takes to reach it.
const SERVES: &str = "tail -f /dev/null";

fn starts_a_server() -> Vec<glave_ports::Completion> {
    vec![
        answer(
            None,
            Some(("bash", json!({"command": SERVES, "background": true}))),
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
        assert_eq!(running[0].command, SERVES);
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

/// How long a command killed by another may take to be recorded as ended: its computer's
/// last moment to read what it printed, and the recording, with room to spare.
const TOLD: Duration = Duration::from_secs(10);

#[test]
fn a_command_killed_by_another_is_recorded_ended_at_once() {
    let dir = folder("bg-api-killed");
    block_on!(async {
        // As `npm run dev` does: the command's own process starts the server, which holds the
        // output open after that process is killed.
        let serve = "sleep 600 & echo $$ > shell.next && mv shell.next shell.pid; wait";
        let kill = "while [ ! -s shell.pid ]; do sleep 0.05; done; kill \"$(cat shell.pid)\"";
        let answers = vec![
            answer(
                None,
                Some(("bash", json!({"command": serve, "background": true}))),
            ),
            answer(None, Some(("bash", json!({"command": kill})))),
            answer(Some("Stopped it."), None),
        ];
        let (server, _) = Setup::new(answers).build(&dir).await;
        let thread = start(&server, false).await;
        let mut watching = server
            .watch(&Caller::default(), thread, None)
            .await
            .unwrap();
        turn(&server, thread, "start the server, then kill it")
            .await
            .unwrap();
        let ended = tokio::time::timeout(TOLD, async {
            loop {
                let Some(Watched::Note(note)) = watching.next().await else {
                    panic!("the watch ended before the command did")
                };
                if let Notification::Event { event, .. } = note.as_ref()
                    && let Body::CommandEnded(ended) = decode(event.clone()).unwrap().body
                {
                    break ended.ending;
                }
            }
        })
        .await
        .expect("the killed command is recorded as ended within seconds");
        assert_eq!(ended, CommandEnding::Signalled);
        assert_eq!(listed(&server, thread).await, []);
    });
}
