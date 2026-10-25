//! A length of time as a person says it, for a message that says how long glave waited.

use std::time::Duration;

/// `length` in words: "10 seconds", "1 minute", "2 hours"; whole minutes or hours when it is
/// one, milliseconds under a second.
pub fn spoken(length: Duration) -> String {
    let secs = length.as_secs();
    let (count, unit) = match secs {
        0 => return format!("{} ms", length.as_millis()),
        secs if secs % 3600 == 0 => (secs / 3600, "hour"),
        secs if secs % 60 == 0 => (secs / 60, "minute"),
        secs => (secs, "second"),
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_length_is_said_in_its_largest_whole_unit() {
        assert_eq!(spoken(Duration::from_secs(10)), "10 seconds");
        assert_eq!(spoken(Duration::from_secs(1)), "1 second");
        assert_eq!(spoken(Duration::from_secs(90)), "90 seconds");
        assert_eq!(spoken(Duration::from_mins(1)), "1 minute");
        assert_eq!(spoken(Duration::from_mins(30)), "30 minutes");
        assert_eq!(spoken(Duration::from_hours(2)), "2 hours");
        assert_eq!(spoken(Duration::from_millis(250)), "250 ms");
    }
}
