//! The server's own sentences an event holds, written again in the caller's language as a
//! transport sends the event: what an approval asks (`approval.requested`'s `ask`), recorded in
//! English, as every sentence is where it is made, written again from the typed `asking` it was
//! written from; why it asks (its `caution`), written again from the typed `reason` it was written from; and
//! who asks and what came of it when a service asks the person, from the service's name; where
//! a fork begins, from how it stands as to files; and what a connector suggested to the person
//! is, from what it was written from.

use std::sync::Arc;

use glave_event::{
    ApprovalRequested, Body, CardLines, ConnectorSuggested, Event, Payload as _, RawEvent,
    ServiceAnswered, ServiceAsked, ServiceOutcome, ThreadForked, decode,
};

use super::{Language, Sentence, caution_in, say, word};
use crate::protocol::{Notification, Update};

/// Whether `event` may hold a sentence of the server's own: an approval does, what a service
/// asks and its answer, where a fork begins, and a connector suggested.
pub(super) fn asks(event: &RawEvent) -> bool {
    [
        ApprovalRequested::TYPE,
        ServiceAsked::TYPE,
        ServiceAnswered::TYPE,
        ThreadForked::TYPE,
        ConnectorSuggested::TYPE,
    ]
    .contains(&event.kind.as_str())
}

/// `event` as a caller who reads `language` is sent it: what an approval asks written in their
/// language, when the table holds the words it was recorded in, and why it asks, when it was
/// recorded with its reason; what a service asks and how it was answered; what a fork tells of
/// its files; what a connector suggested is; `None` when nothing in it changes, as in English.
pub(super) fn event_in(event: &RawEvent, language: Language) -> Option<RawEvent> {
    if language == Language::English || !asks(event) {
        return None;
    }
    let Event { header, body } = decode(event.clone()).ok()?;
    let body = match body {
        Body::ApprovalRequested(mut asked) => {
            // A schedule's card is written again from what it was written from: its question,
            // what that was written from, and its lines.
            if let Some(repeating) = asked.repeating.as_ref() {
                let lines = crate::schedule::card_in(language, repeating);
                asked.asking = Some(crate::schedule::asked_in(language, repeating));
                asked.detail = (lines.as_ref()).map_or(asked.detail, CardLines::joined);
                asked.lines = lines.or(asked.lines);
            }
            let ask = asked.asking.as_ref().map(|ask| word(language, ask));
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
            let service = || asked.service.clone();
            let ask = say(language, &Sentence::ServiceAsks { service: service() });
            let note = if asked.service == glave_harness::ASKED_BY_GLAVE {
                say(language, &Sentence::InstallNote)
            } else {
                say(language, &Sentence::ServiceNote { service: service() })
            };
            asked.ask = ask;
            asked.note = note;
            if let Some(adds) = asked.adds.as_mut().filter(|adds| adds.cut.is_some()) {
                let parts = adds.parts_cut();
                adds.cut = Some(say(language, &Sentence::AddsCut { parts }));
            }
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
        Body::ThreadForked(mut forked) => {
            forked.told = say(language, &crate::fork::forked(forked.files));
            Body::ThreadForked(forked)
        }
        Body::ConnectorSuggested(mut suggested) => {
            suggested.told = word(language, &suggested.suggests);
            Body::ConnectorSuggested(suggested)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_schedule_s_card_is_written_again_its_question_what_that_was_written_from_and_its_lines() {
        let prompt = "Summarize my email and tell me what needs a reply.";
        let raw: RawEvent = serde_json::from_value(serde_json::json!({
            "glave": 1, "id": "0192f5c2-7000-7000-8000-000000000049",
            "thread": "0192f5c0-11aa-7e21-8c3d-5f9b2e4a6c10", "seq": 14,
            "at": "2026-11-16T09:00:00.000Z", "type": "approval.requested", "v": 2,
            "data": {
                "call_id": "call_14", "tool": "schedule", "summary": "schedule: \u{2026}",
                "ask": "Repeat this every weekday at 8:00 am?",
                "asking": {"says": "ask-repeat", "when": "every weekday at 8:00 am"},
                "detail": "\u{2026}", "secret": false,
                "repeating": {
                    "cron": "0 8 * * 1-5", "zone": "Europe/Lisbon", "name": "Morning check",
                    "first": "2026-11-17T08:00:00.000Z", "prompt": prompt
                }
            }
        }))
        .unwrap();
        let written = event_in(&raw, Language::Accented).unwrap();
        let data = serde_json::to_value(&written.data).unwrap();
        assert_eq!(data["asking"]["says"], "ask-repeat");
        let when = data["asking"]["when"].as_str().unwrap();
        assert!(!when.contains("every weekday"), "{when}");
        let ask = data["ask"].as_str().unwrap();
        assert!(!ask.contains("Repeat this") && ask.contains(when), "{ask}");
        let detail = data["detail"].as_str().unwrap();
        assert!(!detail.contains("Monday to Friday"), "{detail}");
        assert!(
            detail.contains(prompt),
            "the model's words stay as they are: {detail}"
        );
        // Its lines, typed, say what the detail says, a line each.
        let lines = &data["lines"];
        let typed: Vec<&str> = ["when", "chat", "prompt"]
            .iter()
            .filter_map(|line| lines[*line].as_str())
            .collect();
        assert_eq!(typed.len(), 3, "{lines}");
        assert_eq!(typed.join("\n"), detail);
    }
}
