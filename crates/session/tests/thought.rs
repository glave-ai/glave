//! A message's thought as it is served: one recorded before glave kept it has its headline read
//! from its reasoning, in a page of `thread/events` and as a watch reads the log, how long it
//! took unknown; the log keeps it as it was recorded.

use std::slice;

use glave_event::{
    At, Body, EventId, Header, MessageAdded, Part, RawEvent, Role, Seq, Thought, decode, encode,
};
use glave_ports::{Caller, EventLog};
use glave_session::protocol::Notification;
use glave_session::{Answering, Watched};
use jiff::Timestamp;

use crate::support::*;

/// The thought of the last `message.added` among `events`.
fn last_thought(events: Vec<RawEvent>) -> Option<Thought> {
    let thoughts: Vec<Option<Thought>> = events
        .into_iter()
        .filter_map(|raw| match decode(raw).unwrap().body {
            Body::MessageAdded(message) => Some(message.thought),
            _ => None,
        })
        .collect();
    thoughts.into_iter().next_back().flatten()
}

#[test]
fn a_message_recorded_before_its_thought_was_kept_is_served_with_its_headline() {
    let dir = folder("thought-old");
    block_on!(async {
        let db = folder("thought-old-db");
        let store = sqlite(&db.join("glave.db")).await;
        let (server, _) = Setup::asking(vec![answer(Some("One."), None)], Answering::NoOne)
            .build_on(&dir, store.log())
            .await;
        let thread = start(&server, false).await;
        turn(&server, thread, "go").await.unwrap();

        // A reply as glave recorded it before it kept a thought: reasoning, and no thought.
        let next = u64::try_from(events(&server, thread).await.len()).unwrap() + 1;
        let at = Timestamp::from_second(1_800_000_000).unwrap();
        let header = Header {
            id: EventId::new(at, [5; 10]).unwrap(),
            thread,
            seq: Seq::new(next).unwrap(),
            at: At::from(at),
        };
        let old = MessageAdded {
            role: Role::Assistant,
            parts: vec![
                Part::Reasoning {
                    text: Some("Something holds the port.\n**Stopping vite**\n".into()),
                    opaque: None,
                },
                Part::Text {
                    text: "Stopped.".into(),
                },
            ],
            author: None,
            thought: None,
        };
        let recorded = encode(header, &old).unwrap();
        store
            .log()
            .append(&scope(), slice::from_ref(&recorded))
            .await
            .unwrap();

        let read = Some(Thought {
            headline: Some("Stopping vite".into()),
            ms: None,
        });
        assert_eq!(last_thought(events(&server, thread).await), read);

        // A watch reads it from the log the same way.
        let mut watch = server
            .watch(&Caller::default(), thread, None)
            .await
            .unwrap();
        let mut watched = Vec::new();
        while watched.len() < usize::try_from(next).unwrap() {
            let Some(Watched::Note(note)) = watch.next().await else {
                panic!("the watch ended early");
            };
            if let Notification::Event { event, .. } = note.as_ref() {
                watched.push(event.clone());
            }
        }
        assert_eq!(last_thought(watched), read);

        // The log keeps it as it was recorded.
        let stored = store.log().read(&scope(), thread, None, 500).await.unwrap();
        assert_eq!(
            stored.last().map(|raw| raw.to_json().unwrap()),
            Some(recorded.to_json().unwrap())
        );
    });
}
