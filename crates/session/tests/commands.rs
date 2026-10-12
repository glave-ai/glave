//! Custom commands through the session API: `turn/start` with `/name arguments` sends the
//! command's text, filled in, as the person's message; anything else is sent as written.

#[cfg(test)]
#[path = "support/model.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod commands {
    use std::path::PathBuf;

    use glave_event::{Body, Part, RawEvent, Role, decode};
    use glave_harness::{Command, Settings};
    use glave_ports::MemoryLog;
    use glave_session::Answering;
    use glave_session::protocol::{Meta, Reply, Request};

    use super::support::*;

    fn user_texts(events: Vec<RawEvent>) -> Vec<String> {
        events
            .into_iter()
            .filter_map(|raw| match decode(raw).unwrap().body {
                Body::MessageAdded(message) if message.role == Role::User => {
                    message.parts.into_iter().find_map(|part| match part {
                        Part::Text { text } => Some(text),
                        _ => None,
                    })
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_turn_that_names_a_command_sends_its_text() {
        let dir = folder("session-commands");
        let mut settings = Settings::new("test-model", "script");
        settings.commands = vec![Command {
            name: "review".into(),
            description: Some("Review a PR".into()),
            argument_hint: None,
            body: "Review PR $1 for bugs.".into(),
            file: PathBuf::from("review.md"),
        }];
        let answers = vec![
            answer(Some("Looks fine."), None),
            answer(Some("Yes."), None),
        ];
        block_on!(async {
            let server = server_with(
                &dir,
                answers,
                None,
                MemoryLog::new(),
                Answering::NoOne,
                1,
                settings,
            )
            .await;
            let Reply::Thread { thread } = server
                .request(Request::ThreadStart {
                    space: None,
                    background: false,
                    worktree: None,
                    computer: None,
                    model: None,
                    approval: None,
                })
                .await
                .unwrap()
            else {
                panic!("expected a thread");
            };
            for prompt in ["/review 12", "/usr/bin is slow"] {
                let turn = Request::TurnStart {
                    background: false,
                    budget: None,
                    thread: thread.id,
                    prompt: prompt.into(),
                    meta: Meta::default(),
                    model: None,
                    approval: None,
                };
                server.request(turn).await.unwrap();
            }
            let page = Request::ThreadEvents {
                thread: thread.id,
                after: None,
                limit: None,
            };
            let Reply::Events { events, .. } = server.request(page).await.unwrap() else {
                panic!("expected events");
            };
            assert_eq!(
                user_texts(events),
                ["Review PR 12 for bugs.", "/usr/bin is slow"]
            );
        });
    }
}
