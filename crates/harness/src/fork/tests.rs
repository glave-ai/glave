use glave_event::{
    At, BlobRef, ModelRef, ParentCall, Producer, ThreadRenamed, ThreadStarted, TurnResult,
    TurnStarted, encode,
};

use super::*;

const HEX: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";

fn header(seq: u64) -> Header {
    let at = At::try_from("2026-11-07T10:00:00.000Z".to_owned()).unwrap();
    let now = jiff::Timestamp::from_second(1_800_000_000).unwrap();
    Header {
        id: EventId::new(now, [u8::try_from(seq).unwrap(); 10]).unwrap(),
        thread: ThreadId::new(now, [7; 10]).unwrap(),
        seq: Seq::new(seq).unwrap(),
        at,
    }
}

fn event(seq: u64, body: Body) -> Event {
    Event {
        header: header(seq),
        body,
    }
}

fn started(parent: Option<ParentCall>) -> Body {
    Body::ThreadStarted(ThreadStarted {
        producer: Producer {
            name: "glave".into(),
            version: "0".into(),
        },
        parent,
        for_: None,
        agent: None,
        origin: Origin::of(OriginKind::Prompt),
    })
}

fn turn_started() -> Body {
    Body::TurnStarted(TurnStarted {
        model: ModelRef {
            provider: "test".into(),
            name: "model".into(),
        },
        config: BlobRef {
            sha256: Sha256::new(HEX).unwrap(),
            bytes: 2,
            media_type: "application/json".into(),
        },
        trace: None,
        auth: None,
        missing: Vec::new(),
    })
}

fn renamed() -> Body {
    Body::ThreadRenamed(ThreadRenamed {
        title: "Retry".into(),
        by: None,
    })
}

/// A thread of two turns, renamed between them, its second still running.
fn thread() -> Vec<Event> {
    vec![
        event(1, started(None)),
        event(2, turn_started()),
        event(3, Body::TurnEnded(TurnResult::begun())),
        event(4, renamed()),
        event(5, turn_started()),
    ]
}

fn seq(n: u64) -> Option<Seq> {
    Seq::new(n).ok()
}

#[test]
fn a_thread_is_forked_only_between_its_turns() {
    let events = thread();
    assert_eq!(
        between_turns(&events, seq(1)),
        Ok(Seq::FIRST),
        "before any turn"
    );
    assert_eq!(
        between_turns(&events, seq(3)).ok(),
        seq(3),
        "as a turn ends"
    );
    assert_eq!(
        between_turns(&events, seq(4)).ok(),
        seq(4),
        "between two turns"
    );
    assert_eq!(between_turns(&events, seq(2)), Err(Invalid::ForkPoint));
    assert_eq!(between_turns(&events, seq(5)), Err(Invalid::ForkPoint));
    assert_eq!(between_turns(&events, seq(9)), Err(Invalid::ForkPoint));
    assert_eq!(
        between_turns(&events, None).ok(),
        seq(4),
        "with no point, the latest before the turn still running"
    );
}

#[test]
fn a_subagents_thread_is_not_forked() {
    let parent = ParentCall {
        thread: header(1).thread,
        call_id: glave_event::CallId::new("call_3").unwrap(),
    };
    let events = vec![event(1, started(Some(parent)))];
    assert_eq!(between_turns(&events, None), Err(Invalid::ForkSubagent));
}

#[test]
fn a_fork_copies_its_history_but_what_is_the_other_threads_own() {
    let kept: Vec<u64> = thread()
        .iter()
        .filter(|event| copied(event))
        .map(|event| event.header.seq.get())
        .collect();
    assert_eq!(kept, [2, 3, 4, 5], "its start is the fork's own");
}

#[test]
fn every_blob_an_event_names_is_found_in_it() {
    let turn = event(2, turn_started()).to_raw().unwrap();
    assert_eq!(blobs_named(&turn), [Sha256::new(HEX).unwrap()]);
    let line = glave_event::MessageAdded {
        role: glave_event::Role::User,
        parts: vec![glave_event::Part::Text {
            text: format!("[image, 2 B, saved as blob:{HEX}] and blob:nothing"),
        }],
        author: None,
        thought: None,
    };
    let message = encode(header(3), &line).unwrap();
    assert_eq!(blobs_named(&message), [Sha256::new(HEX).unwrap()]);
}
