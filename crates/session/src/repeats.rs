//! When a schedule repeats, in the server's words: "every weekday at 8:00 am", on the clocks of
//! the schedule's time zone. The common shapes of a cron expression are said plainly; any other
//! is "on a repeating schedule", never the expression itself, which the person did not type.

use glave_event::When;
use glave_harness::listed;

use crate::cron::{Cron, has};
use crate::words::Sentence;

/// Each weekday's name, Sunday first, as cron numbers them.
const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// The most times in a day said one by one.
const MOST_TIMES: usize = 4;

/// When `when` repeats, as a phrase that opens a line (`start`, "Every weekday at 8:00 am") or
/// follows other words ("every weekday at 8:00 am").
pub(crate) fn repeats(when: &When, start: bool) -> Sentence {
    let start = if start { "yes" } else { "no" }.to_owned();
    let When::Cron(text) = when else {
        return Sentence::RepeatsOnce { start };
    };
    let Ok(cron) = Cron::parse(text) else {
        return Sentence::RepeatsOther { start };
    };
    said(&cron, start.clone()).unwrap_or(Sentence::RepeatsOther { start })
}

/// The plain words for `cron`, when its shape has them.
fn said(cron: &Cron, start: String) -> Option<Sentence> {
    if values(cron.months, 1, 12).len() != 12 {
        return None;
    }
    let minutes = values(cron.minutes, 0, 59);
    let hours = values(cron.hours, 0, 23);
    let every_day = cron.any_day && cron.any_weekday;
    if hours.len() == 24 {
        if !every_day {
            return None;
        }
        if let [past] = minutes.as_slice() {
            return Some(Sentence::RepeatsHourly {
                start,
                past: u64::from(*past),
            });
        }
        let step = minutes.get(1).copied()?;
        let even = step > 0 && 60 % step == 0 && minutes.len() == usize::from(60 / step);
        let from_zero = minutes.iter().zip(0..).all(|(m, i)| *m == step * i);
        return (even && from_zero).then(|| Sentence::RepeatsEveryMinutes {
            start,
            minutes: u64::from(step),
        });
    }
    let [minute] = minutes.as_slice() else {
        return None;
    };
    if hours.len() > MOST_TIMES {
        return None;
    }
    let times: Vec<String> = hours.iter().map(|hour| clock(*hour, *minute)).collect();
    let time = listed(&times);
    let weekdays = values(cron.weekdays, 0, 6);
    if every_day {
        Some(Sentence::RepeatsDaily { start, time })
    } else if cron.any_day && weekdays == [1, 2, 3, 4, 5] {
        Some(Sentence::RepeatsWeekdays { start, time })
    } else if cron.any_day {
        let names: Vec<String> = (weekdays.iter())
            .filter_map(|day| DAYS.get(usize::from(*day)))
            .map(|name| (*name).to_owned())
            .collect();
        let days = listed(&names);
        Some(Sentence::RepeatsWeekly { start, days, time })
    } else if let (true, [day]) = (cron.any_weekday, values(cron.days, 1, 31).as_slice()) {
        let day = u64::from(*day);
        Some(Sentence::RepeatsMonthly { start, day, time })
    } else {
        None
    }
}

/// The values from `low` to `high` that `set` allows.
fn values(set: u64, low: u8, high: u8) -> Vec<u8> {
    (low..=high)
        .filter(|value| i8::try_from(*value).is_ok_and(|value| has(set, value)))
        .collect()
}

/// `hour`:`minute` as a clock in English reads it: `8:00 am`, `12:30 pm`.
fn clock(hour: u8, minute: u8) -> String {
    let (shown, half) = match hour {
        0 => (12, "am"),
        1..=11 => (hour, "am"),
        12 => (12, "pm"),
        _ => (hour.saturating_sub(12), "pm"),
    };
    format!("{shown}:{minute:02} {half}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::words::{Language, say};

    fn line(cron: &str) -> String {
        say(Language::English, &repeats(&When::Cron(cron.into()), true))
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
                "Every Sunday and Saturday at 9:00 am and 5:00 pm",
            ),
            ("0 8 1 * *", "On day 1 of every month at 8:00 am"),
            ("0 * * * *", "Every hour"),
            ("45 * * * *", "Every hour, at 45 minutes past"),
            ("*/15 * * * *", "Every 15 minutes"),
            ("* * * * *", "Every minute"),
            ("0 8 * 6 *", "On a repeating schedule"),
            ("5,7 * * * *", "On a repeating schedule"),
            ("0 1,2,3,4,5 * * *", "On a repeating schedule"),
            ("0 8 13 * 5", "On a repeating schedule"),
        ];
        for (cron, want) in cases {
            assert_eq!(line(cron), want, "{cron}");
        }
        let inside = say(
            Language::English,
            &repeats(&When::Cron("0 8 * * 1-5".into()), false),
        );
        assert_eq!(inside, "every weekday at 8:00 am");
        let once = repeats(
            &When::At(glave_event::At::from(jiff::Timestamp::UNIX_EPOCH)),
            true,
        );
        assert_eq!(say(Language::English, &once), "Once");
    }
}
