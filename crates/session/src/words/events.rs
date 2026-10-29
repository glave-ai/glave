//! The server's own sentences an event holds, written again in the caller's language as a
//! transport sends the event: what an approval asks (`approval.requested`'s `ask`), recorded in
//! English, as every sentence is where it is made, and found again in the table by those words;
//! and why it asks (its `caution`), written again from the typed `reason` it was written from.

use std::sync::Arc;

use glave_event::{ApprovalRequested, Body, Event, Payload as _, RawEvent, decode};

use super::{Language, Sentence, caution_in, say};
use crate::protocol::{Notification, Update};

/// Whether `event` may hold a sentence of the server's own: an approval does.
pub(super) fn asks(event: &RawEvent) -> bool {
    event.kind.as_str() == ApprovalRequested::TYPE
}

/// `event` as a caller who reads `language` is sent it: what an approval asks written in their
/// language, when the table holds the words it was recorded in, and why it asks, when it was
/// recorded with its reason; `None` when nothing in it changes, as in English.
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
    let ask = Sentence::asking(&asked.ask).map(|ask| say(language, &ask));
    let caution = asked
        .reason
        .as_ref()
        .and_then(|reason| caution_in(language, reason));
    if ask.is_none() && caution.is_none() {
        return None;
    }
    asked.ask = ask.unwrap_or(asked.ask);
    asked.caution = caution.or(asked.caution);
    let body = Body::ApprovalRequested(asked);
    Event { header, body }.to_raw().ok()
}

/// `note` as a caller who reads `language` is sent it: an event's sentences, and why a
/// declined call asked, written in their language; any other notification as it is.
pub(crate) fn note_in(note: &Arc<Notification>, language: Language) -> Arc<Notification> {
    match note.as_ref() {
        Notification::Event { thread, event } => {
            if let Some(event) = event_in(event, language) {
                return Arc::new(Notification::Event {
                    thread: *thread,
                    event,
                });
            }
        }
        Notification::Live {
            thread,
            update:
                update @ Update::ToolDeclined {
                    caution: Some(caution),
                    ..
                },
        } if language != Language::English && caution.translatable() => {
            let mut update = update.clone();
            if let Update::ToolDeclined {
                caution: Some(caution),
                ..
            } = &mut update
            {
                caution.translate(language);
            }
            return Arc::new(Notification::Live {
                thread: *thread,
                update,
            });
        }
        _ => {}
    }
    Arc::clone(note)
}
