//! Properties every event must keep, checked on generated input: anything written reads back
//! the same and re-encodes to the same bytes, and no input crashes the reader.

use glave_event::{
    At, Body, EventId, Header, MessageAdded, Part, RawEvent, Role, Seq, ThreadId, decode, encode,
};
use proptest::prelude::*;
use uuid::Uuid;

fn header(seq: u64, millis: u16) -> Header {
    Header {
        id: EventId::try_from(Uuid::parse_str("0192f5c2-7000-7000-8000-000000000001").unwrap())
            .unwrap(),
        thread: ThreadId::try_from(
            Uuid::parse_str("0192f5c0-11aa-7e21-8c3d-5f9b2e4a6c10").unwrap(),
        )
        .unwrap(),
        seq: Seq::new(seq).unwrap(),
        at: At::try_from(format!("2026-10-07T21:14:02.{millis:03}Z")).unwrap(),
    }
}

proptest! {
    /// Any message, whatever its text, decodes to the same message and re-encodes to the
    /// exact bytes stored: the log never drifts as it is read and written again.
    #[test]
    fn any_message_round_trips_byte_for_byte(
        text in "\\PC{0,300}",
        assistant in any::<bool>(),
        seq in 1_u64..u64::MAX / 2,
        millis in 0_u16..1000,
    ) {
        let message = MessageAdded {
            role: if assistant { Role::Assistant } else { Role::User },
            parts: vec![Part::Text { text }],
            author: None,
        };
        let json = encode(header(seq, millis), &message).unwrap().to_json().unwrap();
        let event = decode(RawEvent::from_json(&json).unwrap()).unwrap();
        prop_assert_eq!(event.header.seq, Seq::new(seq).unwrap());
        match &event.body {
            Body::MessageAdded(read) => prop_assert_eq!(read, &message),
            other => prop_assert!(false, "decoded as {:?}", other),
        }
        prop_assert_eq!(event.to_raw().unwrap().to_json().unwrap(), json);
    }

    /// Whatever a line holds, reading it answers or refuses; it never panics.
    #[test]
    fn arbitrary_lines_never_panic(line in "\\PC{0,300}") {
        if let Ok(raw) = RawEvent::from_json(&line) {
            let _ = decode(raw);
        }
    }

    /// Envelopes with any type, version and data are either refused or kept intact.
    #[test]
    fn unknown_or_odd_envelopes_are_kept_or_refused(
        kind in "[a-z]{1,8}(\\.[a-z]{1,8}){0,3}",
        version in 0_u32..5,
        data in "\\{(\"[a-z]{1,5}\":[0-9]{1,3}(,\"[a-z]{1,5}\":[0-9]{1,3}){0,3})?\\}",
    ) {
        let line = format!(
            r#"{{"glave":1,"id":"0192f5c2-7000-7000-8000-000000000001","thread":"0192f5c0-11aa-7e21-8c3d-5f9b2e4a6c10","seq":1,"at":"2026-10-07T21:14:02.400Z","type":"{kind}","v":{version},"data":{data}}}"#
        );
        if let Ok(raw) = RawEvent::from_json(&line)
            && let Ok(event) = decode(raw)
            && matches!(event.body, Body::Unknown(_))
        {
            prop_assert_eq!(event.to_raw().unwrap().to_json().unwrap(), line);
        }
    }
}
