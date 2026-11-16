//! The card the person confirms a schedule with, in the server's words, in their language: the
//! question ("Repeat this every weekday at 8:00 am?"), when it first runs on their clock ("First
//! run: Tuesday 17 November at 8:00 am, your time.", or for a shape with no plain words its
//! next three), and the chat its runs go to. Recorded in English as the approval's `ask` and
//! `detail`, and written again from what it was written from (`repeating`) in the reader's
//! language as a transport sends it.

use glave_event::{Ask, Repeating, When};
use jiff::Timestamp;
use jiff::tz::TimeZone;

use crate::cron::Cron;
use crate::repeats::{clock_in, listed_in, plain, repeats_in};
use crate::words::{Language, Sentence, say};
use crate::zone::time_zone;

/// The runs a card names for a shape with no plain words.
const RUNS_SAID: usize = 3;

/// The first [`RUNS_SAID`] runs of `cron` after `after`, on the clocks of `zone`.
pub(crate) fn first_runs(cron: &Cron, after: Timestamp, zone: &TimeZone) -> Vec<Timestamp> {
    let mut runs = Vec::with_capacity(RUNS_SAID);
    let mut from = after;
    while runs.len() < RUNS_SAID {
        let Some(next) = cron.next_after(from, zone) else {
            break;
        };
        runs.push(next);
        from = next;
    }
    runs
}

/// The card's question for `repeating`, in `language`.
pub(crate) fn asked_in(language: Language, repeating: &Repeating) -> Ask {
    let when = When::Cron(repeating.cron.clone());
    Ask::Repeat {
        when: repeats_in(language, &when, false),
    }
}

/// What the card for `repeating` says below its question, in `language`: when it first runs,
/// and the chat its runs go to; `None` for one that can't be read.
pub(crate) fn card_in(language: Language, repeating: &Repeating) -> Option<String> {
    let clocks = time_zone(&repeating.zone).ok()?;
    let parsed = Cron::parse(&repeating.cron).ok()?;
    let first = repeating.first.timestamp();
    let before = first
        .checked_sub(jiff::SignedDuration::from_millis(1))
        .ok()?;
    let runs: Vec<String> = (first_runs(&parsed, before, &clocks).iter())
        .map(|run| date_in(language, *run, &clocks))
        .collect();
    let runs = if plain(&When::Cron(repeating.cron.clone())) {
        let when = runs.first()?.clone();
        Sentence::RunsFirst { when }
    } else {
        let when = listed_in(language, &runs);
        Sentence::RunsNext { when }
    };
    let name = repeating.name.clone();
    let chat = Sentence::RunsInChat { name };
    Some(format!(
        "{}\n{}",
        say(language, &runs),
        say(language, &chat)
    ))
}

/// `at` on the clocks of `zone`, as a date and time in `language`: `Tuesday 17 November at
/// 8:00 am`.
fn date_in(language: Language, at: Timestamp, zone: &TimeZone) -> String {
    let civil = at.to_zoned(zone.clone());
    let day = u64::from(civil.weekday().to_sunday_zero_offset().unsigned_abs());
    let weekday = say(language, &Sentence::Weekday { day });
    let month = u64::from(civil.month().unsigned_abs());
    let month = say(language, &Sentence::Month { month });
    let (hour, minute) = (civil.hour().unsigned_abs(), civil.minute().unsigned_abs());
    let date = Sentence::DateAt {
        weekday,
        day: u64::from(civil.day().unsigned_abs()),
        month,
        time: clock_in(language, hour, minute),
    };
    say(language, &date)
}

#[cfg(test)]
mod tests {
    use glave_event::At;

    use super::*;

    fn card(cron: &str) -> Repeating {
        Repeating {
            cron: cron.into(),
            zone: "Europe/Lisbon".into(),
            name: "Morning check".into(),
            first: At::try_from("2026-11-17T08:00:00.000Z".to_owned()).unwrap(),
        }
    }

    #[test]
    fn a_card_says_its_first_run_on_the_person_s_clock_and_its_chat_in_their_words() {
        let english = card_in(Language::English, &card("0 8 * * 1-5")).unwrap();
        assert_eq!(
            english,
            "First run: Tuesday 17 November at 8:00 am, your time.\nEach run is added to its own chat, \u{201c}Morning check\u{201d}."
        );
        let other = card_in(Language::English, &card("0 8 */2 * *")).unwrap();
        assert!(
            other.starts_with("Next runs: Tuesday 17 November at 8:00 am, "),
            "{other}"
        );
        assert!(other.contains(" and "), "three runs: {other}");
        let accented = card_in(Language::Accented, &card("0 8 * * 1-5")).unwrap();
        assert!(!accented.contains("First run"), "{accented}");
        let Ask::Repeat { when } = asked_in(Language::Accented, &card("0 8 * * 1-5")) else {
            panic!("a card");
        };
        assert!(
            !when.is_empty() && !when.contains("every weekday"),
            "{when}"
        );
    }
}
