//! What a server that starts after a crash pushes as it closes the turns left open: the chat's
//! end, once, and never its subagents', though it no longer knows which thread started which.

use std::time::Duration;

use glave_event::PrincipalId;
use glave_ports::testing::{FaultyLog, ScriptedModel};
use glave_ports::{InMemoryProjects, Leases, Projects};
use tokio::sync::Notify;

use super::*;

/// The chat calls a subagent, which says "Thought." and then waits in its model call.
fn script() -> Vec<(&'static str, Vec<Completion>)> {
    let brief = json!({"description": "think", "prompt": "CHILD: think"});
    vec![
        (
            "PARENT",
            vec![
                answer(None, Some(("agent", brief))),
                answer(Some("Done."), None),
            ],
        ),
        ("CHILD", vec![answer(Some("Thought."), None)]),
    ]
}

/// A server in `dir` on `log` and `projects`, with leases of its own, as a process of its own
/// has, asking `model`, subagents offered, pushing with `push`.
async fn server<L: EventLog + 'static>(
    dir: &Path,
    log: L,
    projects: &Arc<InMemoryProjects>,
    model: ScriptedModel,
    push: Option<Arc<Pushing>>,
    seed: u64,
) -> Tested<L> {
    // The person each server works for, the same in both, whose projects it closes turns in.
    let person = |n| {
        PrincipalId::new(
            jiff::Timestamp::from_second(1_791_000_000).unwrap(),
            [n; 10],
        )
    };
    let mut setup = Setup::asking(Vec::new(), Answering::Clients);
    setup.model = Some(model);
    setup.seed = seed;
    setup.settings.person = Some(person(1).unwrap());
    setup.settings.agent = Some(person(2).unwrap());
    setup.settings.subagents = true;
    setup.projects = Some(Arc::clone(projects) as Arc<dyn Projects>);
    setup.leases = Some(shared_leases().1 as Arc<dyn Leases>);
    setup.push = push;
    setup.build_on(dir, log).await.0
}

/// Follows `stream`, the chat's, until its subagent has begun its model call: the model streams
/// "Thought." before it waits for its gate.
async fn until_the_subagent_calls(stream: &mut TurnStream) {
    while let Some(item) = stream.next().await {
        if let TurnItem::Note(note) = item
            && let Notification::Live {
                update: Update::Text { text },
                ..
            } = note.as_ref()
            && text == "Thought."
        {
            return;
        }
    }
    panic!("the subagent never began its call");
}

#[test]
fn a_crash_s_turns_closed_as_a_server_starts_push_the_chat_once_and_no_subagent() {
    let dir = folder("push-crash");
    block_on!(async {
        let (service, mut heard) = push_service();
        let (_store, pushes) = sealed();
        let shared = Shared::default();
        let projects = Arc::new(InMemoryProjects::new());
        let log = FaultyLog::over(Arc::clone(&shared.0));
        // Never let go: the subagent is in its call when glave stops.
        let gate = Arc::new(Notify::new());
        let model = ScriptedModel::keyed(script()).gated_for("CHILD", gate);
        let first = server(&dir, log.clone(), &projects, model, None, 1).await;
        let thread = start(&first, false).await;
        let mut stream = stream_turn(&first, thread, "PARENT: go", None)
            .await
            .unwrap();
        until_the_subagent_calls(&mut stream).await;
        // Glave stops at once: nothing more it writes lands, so both turns are left open.
        log.stop_after(0);
        first.stop(Duration::ZERO).await;
        drop((stream, first));

        let model = ScriptedModel::keyed(script());
        let second = server(&dir, shared, &projects, model, Some(pushes), 2).await;
        let phone = Browser::new(12);
        let me = Caller::default();
        subscribe(&second, &me, &phone, &format!("{}/taken", service.url)).await;
        Box::pin(pushing(&second, async {
            let repaired = second.repair_left_open().await.unwrap();
            assert_eq!(repaired.closed, 2, "the chat's turn and its subagent's");
            let said = phone.read(pushed(&mut heard).await.1);
            assert_eq!(
                said["thread"],
                thread.to_string(),
                "the chat, not its subagent"
            );
            assert_eq!(said["kind"], "finished");
            assert!(
                said["text"]
                    .as_str()
                    .unwrap()
                    .starts_with("Didn't finish: ")
            );
        }))
        .await;
        assert!(heard.try_recv().is_err(), "one push, for the chat");
        second.shutdown().await;
    });
}
