//! Five-field cron expressions, as crontab(5) writes them (`minute hour day month weekday`), and
//! the next time one matches, in UTC. Each field takes `*`, a number, a range `a-b`, a step
//! `*/n` or `a-b/n`, or a comma list of those; weekdays run 0 to 7, both Sunday. As in Vixie
//! cron, when both the day and the weekday are restricted, a time matches either.

use jiff::Timestamp;
use jiff::civil::{Date, DateTime};
use jiff::tz::Offset;

use crate::words::Sentence;

/// Days looked ahead for a match: a valid expression that never matches in five years (such
/// as the 31st of February) has no next time.
const LOOKAHEAD_DAYS: u32 = 366 * 5;

/// A parsed cron expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cron {
    minutes: u64,
    hours: u64,
    days: u64,
    months: u64,
    weekdays: u64,
    /// Whether the day field was `*`, so only the weekday restricts days (and the reverse).
    any_day: bool,
    any_weekday: bool,
}

impl Cron {
    /// Parses `text`, or says what is wrong with it.
    pub(crate) fn parse(text: &str) -> Result<Self, Sentence> {
        let fields: Vec<&str> = text.split_whitespace().collect();
        let [minute, hour, day, month, weekday] = fields.as_slice() else {
            return Err(Sentence::CronFields {
                text: text.escape_debug().to_string(),
            });
        };
        let mut weekdays = field(weekday, 0, 7, "weekday")?;
        // 7 is Sunday too.
        if weekdays & (1 << 7) != 0 {
            weekdays |= 1;
        }
        Ok(Self {
            minutes: field(minute, 0, 59, "minute")?,
            hours: field(hour, 0, 23, "hour")?,
            days: field(day, 1, 31, "day")?,
            months: field(month, 1, 12, "month")?,
            weekdays,
            any_day: day.starts_with('*'),
            any_weekday: weekday.starts_with('*'),
        })
    }

    /// The first whole minute strictly after `after` that matches, if one comes within five
    /// years.
    pub(crate) fn next_after(&self, after: Timestamp) -> Option<Timestamp> {
        let start = Offset::UTC.to_datetime(after);
        let mut date = start.date();
        // From the minute after `after`'s minute: seconds never match.
        let (mut hour, mut minute) = (start.hour(), start.minute().saturating_add(1));
        for _ in 0..LOOKAHEAD_DAYS {
            if self.matches_day(date) {
                for h in hour..24 {
                    let from = if h == hour { minute } else { 0 };
                    for m in from..60 {
                        if self.matches_time(h, m) {
                            let at = DateTime::from_parts(date, jiff::civil::time(h, m, 0, 0));
                            return Offset::UTC.to_timestamp(at).ok();
                        }
                    }
                }
            }
            date = date.tomorrow().ok()?;
            (hour, minute) = (0, 0);
        }
        None
    }

    fn matches_time(&self, hour: i8, minute: i8) -> bool {
        has(self.hours, hour) && has(self.minutes, minute)
    }

    fn matches_day(&self, date: Date) -> bool {
        if !has(self.months, date.month()) {
            return false;
        }
        let day = has(self.days, date.day());
        let weekday = has(self.weekdays, date.weekday().to_sunday_zero_offset());
        match (self.any_day, self.any_weekday) {
            (false, false) => day || weekday,
            _ => day && weekday,
        }
    }
}

fn has(set: u64, value: i8) -> bool {
    u32::try_from(value)
        .ok()
        .and_then(|bit| 1u64.checked_shl(bit))
        .is_some_and(|bit| set & bit != 0)
}

/// The values one field allows, as bits; `name` is its name in the expression's own words.
fn field(text: &str, low: u8, high: u8, name: &str) -> Result<u64, Sentence> {
    let bad = || Sentence::CronField {
        field: name.to_owned(),
        text: text.escape_debug().to_string(),
        low,
        high,
    };
    let mut set = 0u64;
    for part in text.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => {
                let step: u8 = step.parse().map_err(|_| bad())?;
                if step == 0 {
                    return Err(bad());
                }
                (range, step)
            }
            None => (part, 1),
        };
        let (from, to) = if range == "*" {
            (low, high)
        } else if let Some((a, b)) = range.split_once('-') {
            (a.parse().map_err(|_| bad())?, b.parse().map_err(|_| bad())?)
        } else {
            let single: u8 = range.parse().map_err(|_| bad())?;
            // `a/n` runs from a to the end, as Vixie cron reads it.
            (single, if part.contains('/') { high } else { single })
        };
        if from < low || to > high || from > to {
            return Err(bad());
        }
        let mut value = from;
        while value <= to {
            set |= 1u64 << value;
            match value.checked_add(step) {
                Some(next) => value = next,
                None => break,
            }
        }
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn next(cron: &str, after: &str) -> Option<String> {
        Cron::parse(cron)
            .unwrap()
            .next_after(after.parse().unwrap())
            .map(|t| t.to_string())
    }

    #[test]
    fn finds_the_next_matching_minute() {
        let cases = [
            ("* * * * *", "2026-10-10T09:00:30Z", "2026-10-10T09:01:00Z"),
            // Strictly after.
            ("0 9 * * *", "2026-10-10T09:00:00Z", "2026-10-11T09:00:00Z"),
            (
                "*/15 * * * *",
                "2026-10-10T09:16:00Z",
                "2026-10-10T09:30:00Z",
            ),
            // 2026-10-10 is a Saturday: weekdays only means Monday.
            (
                "0 9 * * 1-5",
                "2026-10-10T08:00:00Z",
                "2026-10-12T09:00:00Z",
            ),
            (
                "30 23 31 12 *",
                "2026-10-10T00:00:00Z",
                "2026-12-31T23:30:00Z",
            ),
            // Day and weekday both restricted: either matches (the 13th, or a Friday).
            ("0 0 13 * 5", "2026-10-10T00:00:00Z", "2026-10-13T00:00:00Z"),
            // 7 is Sunday.
            ("0 0 * * 7", "2026-10-10T00:00:00Z", "2026-10-11T00:00:00Z"),
            (
                "5,10 1 * * *",
                "2026-10-10T01:05:00Z",
                "2026-10-10T01:10:00Z",
            ),
        ];
        for (cron, after, want) in cases {
            assert_eq!(
                next(cron, after).as_deref(),
                Some(want),
                "{cron} after {after}"
            );
        }
        assert_eq!(next("0 0 31 2 *", "2026-10-10T00:00:00Z"), None);
    }

    #[test]
    fn refuses_what_is_not_cron() {
        for bad in [
            "",
            "* * * *",
            "* * * * * *",
            "60 * * * *",
            "* 24 * * *",
            "* * 0 * *",
            "* * * 13 *",
            "* * * * 8",
            "*/0 * * * *",
            "5-1 * * * *",
            "a * * * *",
            "@daily",
        ] {
            assert!(Cron::parse(bad).is_err(), "{bad:?}");
        }
    }

    proptest! {
        #[test]
        fn the_next_time_is_the_first_matching_minute_after(
            minute in 0u8..60, hour in 0u8..24, step in 1u8..30, weekday in 0u8..8,
            seconds in 1_700_000_000i64..1_900_000_000,
        ) {
            let text = format!("{minute}/{step} {hour} * * {weekday}");
            let cron = Cron::parse(&text).unwrap();
            let after = Timestamp::from_second(seconds).unwrap();
            let found = cron.next_after(after).unwrap();
            prop_assert!(found > after);
            let civil = Offset::UTC.to_datetime(found);
            prop_assert_eq!(civil.second(), 0);
            prop_assert!(cron.matches_time(civil.hour(), civil.minute()));
            prop_assert!(cron.matches_day(civil.date()));
            // No whole minute in between matches.
            let mut probe = Offset::UTC.to_datetime(after).round(jiff::Unit::Minute).unwrap();
            loop {
                let at = Offset::UTC.to_timestamp(probe).unwrap();
                if at >= found {
                    break;
                }
                let matches = at > after
                    && cron.matches_time(probe.hour(), probe.minute())
                    && cron.matches_day(probe.date());
                prop_assert!(!matches, "{} matches before {}", at, found);
                probe = probe.checked_add(jiff::Span::new().minutes(1)).unwrap();
            }
        }
    }
}
