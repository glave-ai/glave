//! The server's own sentences an event holds, written again in the caller's language as a
//! transport sends the event: what an approval asks (`approval.requested`'s `ask`), recorded in
//! English, as every sentence is where it is made, and found again in the table by those words;
//! why it asks (its `caution`), written again from the typed `reason` it was written from; and
//! who asks and what came of it when a service asks the person, from the service's name.

use std::sync::Arc;

use glave_event::{
    ApprovalRequested, Body, Event, Payload as _, RawEvent, ServiceAnswered, ServiceAsked,
    ServiceOutcome, decode,
};

use super::{Language, Sentence, caution_in, say};
use crate::protocol::{Notification, Update};

/// Whether `event` may hold a sentence of the server's own: an approval does, and what a service
/// asks and its answer.
pub(super) fn asks(event: &RawEvent) -> bool {
    [
        ApprovalRequested::TYPE,
        ServiceAsked::TYPE,
        ServiceAnswered::TYPE,
    ]
    .contains(&event.kind.as_str())
}

/// `event` as a caller who reads `language` is sent it: what an approval asks written in their
/// language, when the table holds the words it was recorded in, and why it asks, when it was
/// recorded with its reason; what a service asks and how it was answered; `None` when nothing in
/// it changes, as in English.
pub(super) fn event_in(event: &RawEvent, language: Language) -> Option<RawEvent> {
    if language == Language::English || !asks(event) {
        return None;
    }
    let Event { header, body } = decode(event.clone()).ok()?;
    let body = match body {
        Body::ApprovalRequested(mut asked) => {
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
            Body::ApprovalRequested(asked)
        }
        Body::ServiceAsked(mut asked) => {
            let service = asked.service.clone();
            asked.ask = say(language, &Sentence::ServiceAsks { service });
            Body::ServiceAsked(asked)
        }
        Body::ServiceAnswered(mut answered) => {
            let service = answered.service.clone();
            answered.told = say(
                language,
                &match answered.outcome {
                    ServiceOutcome::Answered => Sentence::ServiceAnswered { service },
                    ServiceOutcome::Declined => Sentence::ServiceDeclined { service },
                    ServiceOutcome::Cancelled => Sentence::ServiceUnanswered { service },
                },
            );
            Body::ServiceAnswered(answered)
        }
        _ => return None,
    };
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
        Notification::Changed { thread, summary } if language != Language::English => {
            let mut summary = summary.clone();
            super::summary_in(&mut summary, language);
            return Arc::new(Notification::Changed {
                thread: *thread,
                summary,
            });
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
