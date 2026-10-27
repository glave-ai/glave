//! The server's own sentences an event holds, written again in the caller's language as a
//! transport sends the event: what an approval asks (`approval.requested`'s `ask`), recorded in
//! English, as every sentence is where it is made, and found again in the table by those words.

use std::sync::Arc;

use glave_event::{ApprovalRequested, Body, Event, Payload as _, RawEvent, decode};

use super::{Language, Sentence, say};
use crate::protocol::Notification;

/// Whether `event` may hold a sentence of the server's own: an approval does.
pub(super) fn asks(event: &RawEvent) -> bool {
    event.kind.as_str() == ApprovalRequested::TYPE
}

/// `event` as a caller who reads `language` is sent it: what an approval asks written in their
/// language, when the table holds the words it was recorded in; `None` when nothing in it
/// changes, as in English.
pub(super) fn event_in(event: &RawEvent, language: Language) -> Option<RawEvent> {
    if language == Language::English || !asks(event) {
        return None;
    }
    let Event {
        header,
        body: Body::ApprovalRequested(mut asked),
    } = decode(event.clone()).ok()?
    else {
        return None;
    };
    asked.ask = say(language, &Sentence::asking(&asked.ask)?);
    let body = Body::ApprovalRequested(asked);
    Event { header, body }.to_raw().ok()
}

/// `note` as a caller who reads `language` is sent it: an event's sentences written in their
/// language; any other notification as it is.
pub(crate) fn note_in(note: &Arc<Notification>, language: Language) -> Arc<Notification> {
    if let Notification::Event { thread, event } = note.as_ref()
        && let Some(event) = event_in(event, language)
    {
        return Arc::new(Notification::Event {
            thread: *thread,
            event,
        });
    }
    Arc::clone(note)
}
