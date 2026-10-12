//! Custom commands through the session API: `turn/start` with `/name arguments` sends the
//! command's text, filled in, as the person's message; anything else is sent as written.

use std::path::PathBuf;

use glave_event::{Body, Part, RawEvent, Role, decode};
use glave_harness::Command;
use glave_session::Answering;

use crate::support::*;

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
    let mut setup = Setup::new(vec![
        answer(Some("Looks fine."), None),
        answer(Some("Yes."), None),
    ]);
    setup.answering = Answering::NoOne;
    setup.settings.commands = vec![Command {
        name: "review".into(),
        description: Some("Review a PR".into()),
        argument_hint: None,
        body: "Review PR $1 for bugs.".into(),
        file: PathBuf::from("review.md"),
    }];
    block_on!(async {
        let (server, _) = setup.build(&dir).await;
        let thread = start(&server, false).await;
        for prompt in ["/review 12", "/usr/bin is slow"] {
            turn(&server, thread, prompt).await.unwrap();
        }
        assert_eq!(
            user_texts(events(&server, thread).await),
            ["Review PR 12 for bugs.", "/usr/bin is slow"]
        );
    });
}
