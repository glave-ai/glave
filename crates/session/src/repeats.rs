//! When a schedule repeats, in the server's words: "every weekday at 8:00 am", on the clocks of
//! the schedule's time zone, in the reader's language. Every word, a weekday's name and a
//! time's included, is a message of the catalog (`words.rs`); the phrase is put together from
//! them in the language it is written in. The common shapes of a cron expression are said
//! plainly, read from the days and times it matches, never from how its fields were written;
//! any other is "on a repeating schedule", never the expression, which the person did not type.

use glave_event::When;

use crate::cron::{Cron, has};
use crate::words::{Language, Sentence, say};

/// The most times in a day said one by one.
const MOST_TIMES: usize = 4;

/// The last day every month has: a later one is skipped in the months without it, which plain
/// words for it would hide.
const EVERY_MONTH_HAS: u8 = 28;

/// What a schedule's days come to.
pub(crate) enum Days {
    Every,
    Weekdays,
    /// These weekdays, Monday first, Sunday as 0.
    Weekly(Vec<u8>),
    /// This day of every month, one every month has.
    Monthly(u8),
}

/// A cron expression's shape, when it has plain words.
pub(crate) enum Shape {
    /// Every hour, at `past` minutes past.
    Hourly { past: u8 },
    /// Every `minutes` minutes, from the hour.
    EveryMinutes { minutes: u8 },
    /// On `days`, at each of `hours`, `minute` past.
    OnDays {
        days: Days,
        hours: Vec<u8>,
        minute: u8,
    },
}

/// When `when` repeats, written in `language`, as a phrase that opens a line (`start`, "Every
/// weekday at 8:00 am") or follows other words ("every weekday at 8:00 am").
pub(crate) fn repeats_in(language: Language, when: &When, start: bool) -> String {
    let start = if start { "yes" } else { "no" }.to_owned();
    let said = match when {
        When::At(_) => Some(Sentence::RepeatsOnce {
            start: start.clone(),
        }),
        When::Cron(text) => Cron::parse(text)
            .ok()
            .and_then(|cron| plainly(language, &cron, start.clone())),
    };
    say(language, &said.unwrap_or(Sentence::RepeatsOther { start }))
}

/// The plain words for `cron` in `language`, when its shape has them.
fn plainly(language: Language, cron: &Cron, start: String) -> Option<Sentence> {
    Some(match shape_of(cron)? {
        Shape::Hourly { past } => Sentence::RepeatsHourly {
            start,
            past: u64::from(past),
        },
        Shape::EveryMinutes { minutes } => Sentence::RepeatsEveryMinutes {
            start,
            minutes: u64::from(minutes),
        },
        Shape::OnDays {
            days,
            hours,
            minute,
        } => {
            let time = times_in(language, &hours, minute);
            match days {
                Days::Every => Sentence::RepeatsDaily { start, time },
                Days::Weekdays => Sentence::RepeatsWeekdays { start, time },
                Days::Weekly(weekdays) => Sentence::RepeatsWeekly {
                    start,
                    days: weekdays_in(language, &weekdays),
                    time,
                },
                Days::Monthly(day) => Sentence::RepeatsMonthly {
                    start,
                    day: u64::from(day),
                    time,
                },
            }
        }
    })
}

/// The shape of `cron`, when it has plain words: every month, and either every day all day
/// on the hour or evenly through it, or a few times a day on days that say plainly.
pub(crate) fn shape_of(cron: &Cron) -> Option<Shape> {
    if values(cron.months, 1, 12).len() != 12 {
        return None;
    }
    let minutes = values(cron.minutes, 0, 59);
    let hours = values(cron.hours, 0, 23);
    let days = days_of(cron)?;
    if hours.len() == 24 {
        if !matches!(days, Days::Every) {
            return None;
        }
        if let [past] = minutes.as_slice() {
            return Some(Shape::Hourly { past: *past });
        }
        let step = minutes.get(1).copied()?;
        let even = step > 0 && 60 % step == 0 && minutes.len() == usize::from(60 / step);
        let from_zero = minutes.iter().zip(0..).all(|(m, i)| *m == step * i);
        return (even && from_zero).then_some(Shape::EveryMinutes { minutes: step });
    }
    let [minute] = minutes.as_slice() else {
        return None;
    };
    (hours.len() <= MOST_TIMES).then_some(Shape::OnDays {
        days,
        hours,
        minute: *minute,
    })
}

/// What the days `cron` matches come to, read from the sets as cron matches them: when either
/// field is written with `*`, a day must be in both; when both are restricted, in either.
fn days_of(cron: &Cron) -> Option<Days> {
    let days = values(cron.days, 1, 31);
    let mut weekdays = values(cron.weekdays, 0, 6);
    // Monday first: Sunday ends the week.
    weekdays.sort_by_key(|day| (day + 6) % 7);
    let every_date = days.len() == 31;
    let every_weekday = weekdays.len() == 7;
    let either = !cron.any_day && !cron.any_weekday;
    if either {
        return (every_date || every_weekday).then_some(Days::Every);
    }
    match (every_date, every_weekday, days.as_slice()) {
        (true, true, _) => Some(Days::Every),
        (true, false, _) if weekdays == [1, 2, 3, 4, 5] => Some(Days::Weekdays),
        (true, false, _) => Some(Days::Weekly(weekdays)),
        (false, true, [day]) if *day <= EVERY_MONTH_HAS => Some(Days::Monthly(*day)),
        _ => None,
    }
}

/// The values from `low` to `high` that `set` allows.
fn values(set: u64, low: u8, high: u8) -> Vec<u8> {
    (low..=high)
        .filter(|value| i8::try_from(*value).is_ok_and(|value| has(set, value)))
        .collect()
}

/// `days`, as a line opens with them, in `language`: "Monday to Friday".
pub(crate) fn days_in(language: Language, days: &Days) -> String {
    match days {
        Days::Every => say(language, &Sentence::DaysEvery),
        Days::Weekdays => say(language, &Sentence::DaysWeekdays),
        Days::Weekly(weekdays) => weekdays_in(language, weekdays),
        Days::Monthly(day) => say(
            language,
            &Sentence::DaysMonthly {
                day: u64::from(*day),
            },
        ),
    }
}

/// `weekdays`, Sunday as 0, by name in a list, in `language`: "Monday and Friday".
fn weekdays_in(language: Language, weekdays: &[u8]) -> String {
    let names: Vec<String> = (weekdays.iter())
        .map(|day| {
            let day = u64::from(*day);
            say(language, &Sentence::Weekday { day })
        })
        .collect();
    listed_in(language, &names)
}

/// Each of `hours` at `minute` past, in a list, in `language`: "9:00 am and 5:00 pm".
pub(crate) fn times_in(language: Language, hours: &[u8], minute: u8) -> String {
    let times: Vec<String> = (hours.iter())
        .map(|hour| clock_in(language, *hour, minute))
        .collect();
    listed_in(language, &times)
}

/// `hour`:`minute` on a clock, in `language`: `8:00 am`.
pub(crate) fn clock_in(language: Language, hour: u8, minute: u8) -> String {
    let half = if hour < 12 { "am" } else { "pm" }.to_owned();
    let hour = match hour % 12 {
        0 => 12,
        other => other,
    };
    let clock = Sentence::Clock {
        hour: u64::from(hour),
        minute: format!("{minute:02}"),
        half,
    };
    say(language, &clock)
}

/// `items` as a list in a sentence, in `language`: `a`, `a and b`, `a, b and c`.
pub(crate) fn listed_in(language: Language, items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [first, rest @ .., last] => {
            let all = (rest.iter()).fold(first.clone(), |all, next| {
                let next = next.clone();
                say(language, &Sentence::ListComma { all, next })
            });
            let last = last.clone();
            say(language, &Sentence::ListAnd { all, last })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(cron: &str) -> String {
        repeats_in(Language::English, &When::Cron(cron.into()), true)
    }

    #[test]
    fn common_shapes_are_said_plainly_and_the_rest_without_the_expression() {
        let cases = [
            ("0 8 * * 1-5", "Every weekday at 8:00 am"),
            ("30 19 * * *", "Every day at 7:30 pm"),
            ("0 0 * * *", "Every day at 12:00 am"),
            ("0 12 * * 0", "Every Sunday at 12:00 pm"),
            (
                "15 9 * * 1,3,5",
                "Every Monday, Wednesday and Friday at 9:15 am",
            ),
            (
                "0 9,17 * * 6,7",
                "Every Saturday and Sunday at 9:00 am and 5:00 pm",
            ),
            ("0 9 * * 0,6", "Every Saturday and Sunday at 9:00 am"),
            ("0 9 * * 0,1", "Every Monday and Sunday at 9:00 am"),
            ("0 8 1 * *", "On day 1 of every month at 8:00 am"),
            ("0 8 28 * *", "On day 28 of every month at 8:00 am"),
            // Not every month has the 29th to the 31st: its next runs say when instead.
            ("0 8 29 * *", "On a repeating schedule"),
            ("0 8 31 * *", "On a repeating schedule"),
            ("0 8 1-31 * *", "Every day at 8:00 am"),
            ("0 8 * * 0-6", "Every day at 8:00 am"),
            ("0 * * * *", "Every hour"),
            ("45 * * * *", "Every hour, at 45 minutes past"),
            ("*/15 * * * *", "Every 15 minutes"),
            ("* * * * *", "Every minute"),
            ("0 8 * 6 *", "On a repeating schedule"),
            ("5,7 * * * *", "On a repeating schedule"),
            ("0 1,2,3,4,5 * * *", "On a repeating schedule"),
            ("0 8 13 * 5", "On a repeating schedule"),
            ("0 8 */2 * *", "On a repeating schedule"),
            ("0 8 */2 * 1-5", "On a repeating schedule"),
        ];
        for (cron, want) in cases {
            assert_eq!(line(cron), want, "{cron}");
        }
        let inside = repeats_in(Language::English, &When::Cron("0 8 * * 1-5".into()), false);
        assert_eq!(inside, "every weekday at 8:00 am");
        let at = When::At(glave_event::At::from(jiff::Timestamp::UNIX_EPOCH));
        assert_eq!(repeats_in(Language::English, &at, true), "Once");
    }

    #[test]
    fn every_word_of_it_is_the_catalog_s_in_the_reader_s_language() {
        let accented = repeats_in(
            Language::Accented,
            &When::Cron("0 9,17 * * 6,7".into()),
            true,
        );
        // Not one plain English letter is left: each name, time and joining word went through
        // the catalog.
        for word in ["Sunday", "Saturday", "and", "am", "pm", "Every"] {
            assert!(!accented.contains(word), "{word} in {accented}");
        }
        assert!(
            accented.contains('9') && accented.contains("00"),
            "{accented}"
        );
    }
}
