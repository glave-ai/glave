//! Five-field cron expressions, as crontab(5) writes them (`minute hour day month weekday`), and
//! the next time one matches on the clocks of a time zone. Each field takes `*`, a number, a
//! range `a-b`, a step `*/n` or `a-b/n`, or a comma list of those; weekdays run 0 to 7, both
//! Sunday. As in Vixie cron, when both the day and the weekday are restricted, a time matches
//! either. Across daylight saving, a time the clocks skip runs as they reach it, an hour on
//! (jiff's compatible reading), and a time they repeat runs once, the first time.

use jiff::Timestamp;
use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;

use crate::words::Sentence;

/// Days looked ahead for a match: a valid expression that never matches in five years (such
/// as the 31st of February) has no next time.
const LOOKAHEAD_DAYS: u32 = 366 * 5;

/// Matching times on the clocks looked at for one after a moment: as the clocks go back, the
/// times of the hour they repeat come before it, once each.
const MOST_REPEATED: usize = 24 * 60;

/// The fewest minutes a schedule leaves between two runs: each run is its maker's model calls,
/// whose credits and budgets cap what one spends, and this caps how often.
pub(crate) const SHORTEST_GAP: u16 = 5;

const MINUTES_A_DAY: u32 = 24 * 60;

/// A parsed cron expression: the values each field allows, as bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cron {
    pub(crate) minutes: u64,
    pub(crate) hours: u64,
    pub(crate) days: u64,
    pub(crate) months: u64,
    pub(crate) weekdays: u64,
    /// Whether the day field was `*`, so only the weekday restricts days (and the reverse).
    pub(crate) any_day: bool,
    pub(crate) any_weekday: bool,
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

    /// The first whole minute strictly after `after` that matches on the clocks of `zone`, if one
    /// comes within five years.
    pub(crate) fn next_after(&self, after: Timestamp, zone: &TimeZone) -> Option<Timestamp> {
        let mut from = zone.to_datetime(after);
        for _ in 0..MOST_REPEATED {
            let civil = self.next_on_clocks(from)?;
            let at = zone.to_ambiguous_timestamp(civil).compatible().ok()?;
            if at > after {
                return Some(at);
            }
            // A time of an hour the clocks repeat, which ran the first time.
            from = civil;
        }
        None
    }

    /// The first whole minute on the clocks strictly after `start`'s minute that matches.
    fn next_on_clocks(&self, start: DateTime) -> Option<DateTime> {
        let mut date = start.date();
        // From the minute after `after`'s minute: seconds never match.
        let (mut hour, mut minute) = (start.hour(), start.minute().saturating_add(1));
        for _ in 0..LOOKAHEAD_DAYS {
            if self.matches_day(date) {
                for h in hour..24 {
                    let from = if h == hour { minute } else { 0 };
                    for m in from..60 {
                        if self.matches_time(h, m) {
                            return Some(DateTime::from_parts(date, jiff::civil::time(h, m, 0, 0)));
                        }
                    }
                }
            }
            date = date.tomorrow().ok()?;
            (hour, minute) = (0, 0);
        }
        None
    }

    /// `text` parsed for a schedule, which also never runs twice within [`SHORTEST_GAP`]
    /// minutes: refused, in plain words, when it would.
    pub(crate) fn for_a_schedule(text: &str) -> Result<Self, Sentence> {
        let cron = Self::parse(text)?;
        if cron.shortest_gap() < u32::from(SHORTEST_GAP) {
            return Err(Sentence::CronTooOften {
                minutes: SHORTEST_GAP,
            });
        }
        Ok(cron)
    }

    /// The fewest minutes between two of its times on the clocks: within a day, and from a
    /// day's last to the next day's first, as if every day matched.
    pub(crate) fn shortest_gap(&self) -> u32 {
        let times: Vec<u32> = (0..24i8)
            .flat_map(|hour| (0..60i8).map(move |minute| (hour, minute)))
            .filter(|(hour, minute)| self.matches_time(*hour, *minute))
            .filter_map(|(hour, minute)| {
                let (hour, minute) = (u32::try_from(hour).ok()?, u32::try_from(minute).ok()?);
                Some(hour * 60 + minute)
            })
            .collect();
        let overnight =
            (times.first().zip(times.last())).map(|(first, last)| first + MINUTES_A_DAY - last);
        (times.iter().zip(times.iter().skip(1)))
            .map(|(earlier, later)| later - earlier)
            .chain(overnight)
            .min()
            .unwrap_or(MINUTES_A_DAY)
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

pub(crate) fn has(set: u64, value: i8) -> bool {
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
        next_in(cron, after, "UTC")
    }

    fn next_in(cron: &str, after: &str, zone: &str) -> Option<String> {
        let zone = TimeZone::get(zone).unwrap();
        Cron::parse(cron)
            .unwrap()
            .next_after(after.parse().unwrap(), &zone)
            .map(|t| t.to_string())
    }

    #[test]
    fn times_are_read_on_the_clocks_of_the_zone() {
        // 2026-11-16 is a Monday; Lisbon keeps UTC in winter and UTC+1 in summer.
        let lisbon = |after| next_in("0 8 * * 1-5", after, "Europe/Lisbon");
        assert_eq!(
            lisbon("2026-11-16T09:00:00Z").as_deref(),
            Some("2026-11-17T08:00:00Z")
        );
        assert_eq!(
            lisbon("2026-07-01T09:00:00Z").as_deref(),
            Some("2026-07-02T07:00:00Z")
        );
        let tokyo = next_in("30 7 * * *", "2026-11-16T00:00:00Z", "Asia/Tokyo");
        assert_eq!(tokyo.as_deref(), Some("2026-11-16T22:30:00Z"));
    }

    #[test]
    fn across_daylight_saving_a_skipped_time_runs_an_hour_on_and_a_repeated_one_runs_once() {
        let york = |cron, after| next_in(cron, after, "America/New_York");
        // 2026-03-08: 2:00 becomes 3:00, so 2:30 never shows; it runs at 3:30 EDT.
        assert_eq!(
            york("30 2 * * *", "2026-03-07T12:00:00Z").as_deref(),
            Some("2026-03-08T07:30:00Z")
        );
        assert_eq!(
            york("30 2 * * *", "2026-03-08T07:30:00Z").as_deref(),
            Some("2026-03-09T06:30:00Z")
        );
        // 2026-11-01: 2:00 goes back to 1:00, so 1:30 shows twice; it runs the first time.
        let first = york("30 1 * * *", "2026-10-31T12:00:00Z");
        assert_eq!(first.as_deref(), Some("2026-11-01T05:30:00Z"));
        assert_eq!(
            york("30 1 * * *", "2026-11-01T05:30:00Z").as_deref(),
            Some("2026-11-02T06:30:00Z"),
            "not again at 1:30 EST"
        );
        // Every 20 minutes through the repeated hour: each time on the clocks once.
        assert_eq!(
            york("*/20 * * * *", "2026-11-01T05:40:00Z").as_deref(),
            Some("2026-11-01T07:00:00Z"),
            "1:00 to 1:40 EST were run as EDT; next is 2:00 EST"
        );
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

    #[test]
    fn a_schedule_never_runs_twice_within_five_minutes_on_any_day_s_clocks() {
        let gap = |text| Cron::parse(text).unwrap().shortest_gap();
        assert_eq!(gap("* * * * *"), 1);
        assert_eq!(gap("*/2 * * * *"), 2);
        assert_eq!(gap("0,4 9 * * *"), 4);
        assert_eq!(gap("*/5 * * * *"), 5);
        assert_eq!(gap("0 9 * * 1-5"), 24 * 60);
        // From one day's last time to the next day's first.
        assert_eq!(gap("0,58 0,23 * * *"), 2);
        for often in ["* * * * *", "*/2 * * * *", "0,4 9 * * *", "0,58 0,23 * * *"] {
            assert_eq!(
                Cron::for_a_schedule(often),
                Err(Sentence::CronTooOften { minutes: 5 }),
                "{often}"
            );
        }
        for enough in ["*/5 * * * *", "0 8 * * 1-5", "30 7 1 * *"] {
            assert!(Cron::for_a_schedule(enough).is_ok(), "{enough}");
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
            let found = cron.next_after(after, &TimeZone::UTC).unwrap();
            prop_assert!(found > after);
            let civil = jiff::tz::Offset::UTC.to_datetime(found);
            prop_assert_eq!(civil.second(), 0);
            prop_assert!(cron.matches_time(civil.hour(), civil.minute()));
            prop_assert!(cron.matches_day(civil.date()));
            // No whole minute in between matches.
            let mut probe = jiff::tz::Offset::UTC.to_datetime(after).round(jiff::Unit::Minute).unwrap();
            loop {
                let at = jiff::tz::Offset::UTC.to_timestamp(probe).unwrap();
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
