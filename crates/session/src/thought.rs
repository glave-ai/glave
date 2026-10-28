//! A message's thought as it is served (`specs/session.md`, "Notifications"): one recorded before
//! glave kept it has its headline read from its reasoning as it is served, how long it took
//! unknown, so a client never shows a bare "Thought". What is stored stays as it is.

use glave_event::{Body, Event, MessageAdded, Payload as _, RawEvent, decode, encode};
use glave_harness::thought_of;

/// `raw` as it is served: a message with reasoning and no thought gets the one its reasoning
/// reads as; anything else, as it is.
pub(crate) fn served(raw: RawEvent) -> RawEvent {
    // Only a message's own words can hold reasoning: anything else passes unread.
    if raw.kind.as_str() != MessageAdded::TYPE || !raw.data.get().contains("\"reasoning\"") {
        return raw;
    }
    let Ok(Event {
        header,
        body: Body::MessageAdded(mut message),
    }) = decode(raw.clone())
    else {
        return raw;
    };
    if message.thought.is_some() {
        return raw;
    }
    let Some(thought) = thought_of(&message.parts) else {
        return raw;
    };
    message.thought = Some(thought);
    encode(header, &message).unwrap_or(raw)
}

#[cfg(test)]
mod tests {
    use glave_event::Thought;

    use super::*;

    fn message(data: &str) -> RawEvent {
        let line = format!(
            r#"{{"glave":1,"id":"0190a0b0-0000-7000-8000-000000000001","thread":"0190a0b0-0000-7000-8000-000000000002","seq":2,"at":"2026-10-28T00:00:00Z","type":"message.added","v":1,"data":{data}}}"#
        );
        RawEvent::from_json(&line).unwrap_or_else(|error| panic!("{error}"))
    }

    fn thought(raw: RawEvent) -> Option<Thought> {
        match decode(raw).map(|event| event.body) {
            Ok(Body::MessageAdded(message)) => message.thought,
            _ => None,
        }
    }

    #[test]
    fn an_old_message_with_reasoning_reads_its_headline_and_no_time() {
        let old = message(
            r#"{"role":"assistant","parts":[{"type":"reasoning","text":"The port is taken."},{"type":"text","text":"Done."}]}"#,
        );
        assert_eq!(
            thought(served(old)),
            Some(Thought {
                headline: Some("The port is taken.".to_owned()),
                ms: None,
            })
        );
    }

    #[test]
    fn a_message_with_a_thought_or_no_reasoning_is_served_as_recorded() {
        for data in [
            r#"{"role":"assistant","parts":[{"type":"reasoning","text":"A."}],"thought":{"headline":"B.","ms":5}}"#,
            r#"{"role":"assistant","parts":[{"type":"text","text":"Plain \"reasoning\"."}]}"#,
            r#"{"role":"assistant","parts":[{"type":"reasoning","text":"  "}]}"#,
        ] {
            let raw = message(data);
            assert_eq!(
                served(raw.clone()).to_json().ok(),
                raw.to_json().ok(),
                "{data}"
            );
        }
    }
}
