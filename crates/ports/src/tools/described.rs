//! A call in words: in a line, for the person following along, and in full, as an approval of
//! it shows it to the person and to the judge, every key of its arguments there.

use glave_event::Arguments;
use serde_json::Value;

/// How much of a call's arguments its line shows.
const DESCRIBED: usize = 120;

/// The most of a call's arguments an approval shows, in characters: every key, and as much of
/// each value as fits.
const IN_FULL: usize = 4_000;

/// How much of one long value an approval shows, tried in turn until every key fits.
const VALUE_ROOMS: [usize; 4] = [600, 200, 60, 20];

/// A call in a few words: the tool's name, then its arguments cut short.
pub fn describe_call(name: &str, arguments: &Arguments) -> String {
    let text = match arguments {
        Arguments::Json(map) => serde_json::to_string(map).unwrap_or_default(),
        Arguments::Malformed(raw) => raw.clone(),
    };
    let short: String = text.chars().take(DESCRIBED).collect();
    let more = if text.chars().count() > DESCRIBED {
        "…"
    } else {
        ""
    };
    format!("{name} {short}{more}")
}

/// A call whole, as an approval shows it: the tool's name, then every key of its arguments, a
/// value too long to show whole shortened in the middle with how much was left out, within a
/// few thousand characters. What is cut is never a key at the end, as a line's cut would be.
pub fn describe_call_in_full(name: &str, arguments: &Arguments) -> String {
    let text = match arguments {
        Arguments::Json(map) => {
            let whole = Value::Object(map.clone());
            let mut written = String::new();
            for room in VALUE_ROOMS {
                written = serde_json::to_string(&shortened(&whole, room)).unwrap_or_default();
                if written.chars().count() <= IN_FULL {
                    break;
                }
            }
            middle(&written, IN_FULL)
        }
        Arguments::Malformed(raw) => middle(raw, IN_FULL),
    };
    format!("{name} {text}")
}

/// `value` with every text in it, keys too, within about `room` characters.
fn shortened(value: &Value, room: usize) -> Value {
    match value {
        Value::String(text) => Value::String(middle(text, room)),
        Value::Array(items) => Value::Array(items.iter().map(|it| shortened(it, room)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, it)| (middle(key, room), shortened(it, room)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// `text` within about `room` characters: whole when it fits, else its start and its end, with
/// how many characters between them were left out.
fn middle(text: &str, room: usize) -> String {
    let count = text.chars().count();
    if count <= room {
        return text.to_owned();
    }
    let head = room.saturating_mul(2) / 3;
    let tail = room.saturating_sub(head);
    let left = count.saturating_sub(head).saturating_sub(tail);
    let start: String = text.chars().take(head).collect();
    let end: String = text.chars().skip(count.saturating_sub(tail)).collect();
    format!("{start}…[{left} characters left out]…{end}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn arguments(value: Value) -> Arguments {
        Arguments::Json(serde_json::from_value(value).unwrap())
    }

    #[test]
    fn in_full_every_key_shows_after_a_long_value() {
        let body = "Hello. ".repeat(43);
        let call = arguments(json!({
            "to": "ana@example.com", "body": body, "cc": "drop@collector.example"
        }));
        let line = describe_call("mcp__gmail__send_message", &call);
        assert!(!line.contains("drop@collector.example"), "a line is cut");
        let full = describe_call_in_full("mcp__gmail__send_message", &call);
        assert!(full.starts_with("mcp__gmail__send_message {"), "{full}");
        assert!(full.contains(r#""cc":"drop@collector.example""#), "{full}");
        assert!(full.contains(&body), "a value that fits shows whole");
    }

    #[test]
    fn a_long_value_is_shortened_in_the_middle_and_the_whole_stays_bounded() {
        let long = format!("{}{}", "a".repeat(10_000), "planted-at-the-end");
        let call = arguments(json!({ "body": long, "to": "ana@example.com" }));
        let full = describe_call_in_full("send", &call);
        assert!(full.contains("characters left out]…"), "{full}");
        assert!(full.contains("planted-at-the-end"), "its end shows");
        assert!(full.contains("ana@example.com"));
        let many: serde_json::Map<String, Value> = (0..2_000)
            .map(|n| (format!("key{n:04}"), json!("x".repeat(50))))
            .collect();
        let crowded = describe_call_in_full("send", &Arguments::Json(many));
        assert!(crowded.chars().count() < IN_FULL + 100, "bounded");
        assert!(crowded.contains("key1999"), "the last key shows: {crowded}");
        let raw = describe_call_in_full("send", &Arguments::Malformed("y".repeat(9_000)));
        assert!(raw.chars().count() < IN_FULL + 100);
    }
}
