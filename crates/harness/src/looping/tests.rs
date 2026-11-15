use std::fmt::Write as _;

use super::*;

/// The end of a reply DeepSeek V4.1 Flash repeated for 195 KB until a deadline stopped it.
const EMIT_OK: &str = include_str!("emit_ok.txt");
/// The end of another, from another task.
const EMIT_DONE: &str = include_str!("emit_done.txt");

/// Whether `text`, streamed in pieces of `size` characters, is seen to repeat.
fn repeats(text: &str, size: usize) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let mut looping = Looping::default();
    chars.chunks(size).any(|piece| {
        let piece: String = piece.iter().collect();
        looping.see(&piece)
    })
}

/// A reply that answers properly before `then`.
fn answered_then(then: &str) -> String {
    format!(
        "The codename of the 3.1 release is **Saffron Marten**, as its release notes say.\n\n\
         That page was the only source I found.\n\n{then}"
    )
}

#[test]
fn the_loops_that_ran_to_a_deadline_are_seen_early() {
    for (name, text) in [("emit/ok", EMIT_OK), ("emit/done", EMIT_DONE)] {
        let reply = answered_then(&text.repeat(4));
        assert!(repeats(&reply, 7), "{name} was not seen");
        // Seen within its first 2,000 characters, some 500 tokens, of 195,000.
        let mut looping = Looping::default();
        let at = reply
            .char_indices()
            .find(|(at, _)| looping.see(reply.get(*at..*at + 1).unwrap_or("")))
            .map(|(at, _)| at);
        assert!(at.is_some_and(|at| at < 2_000), "{name} seen at {at:?}");
    }
}

#[test]
fn a_loop_on_one_line_is_seen_too() {
    assert!(repeats(&answered_then(&"Emit. Done. Ok. ".repeat(60)), 3));
}

#[test]
fn a_long_table_is_not_a_loop() {
    let mut table =
        String::from("| check | linux | macos | windows |\n| --- | --- | --- | --- |\n");
    for row in 0..200 {
        let _ = writeln!(table, "| check {row} | passed | passed | passed |");
    }
    // Identical rows too, as a template to fill in.
    for _ in 0..40 {
        table.push_str("|  |  |  |  |\n");
    }
    assert!(!repeats(&answered_then(&table), 11));
}

#[test]
fn repeated_lines_of_code_are_not_a_loop() {
    let mut code = String::from("Here are the tests:\n\n```rust\n");
    for _ in 0..60 {
        code.push_str("    assert_eq!(parse(\"a = 1\"), Ok(Value::Int(1)));\n");
    }
    code.push_str("```\n\nAll of them pass.\n");
    assert!(!repeats(&answered_then(&code), 13));
}

#[test]
fn a_list_whose_items_begin_alike_is_not_a_loop() {
    let mut list = String::new();
    for item in 1..=120 {
        let _ = writeln!(
            list,
            "{item}. Read the configuration file and check that every key in it is valid ({item})"
        );
    }
    for name in [
        "parser", "lexer", "printer", "loader", "runner", "checker", "linker",
    ] {
        for step in 0..3 {
            let _ = writeln!(
                list,
                "- Fixed a bug where the {name} could stop before the end of its input, step {step}"
            );
        }
    }
    assert!(!repeats(&answered_then(&list), 5));
}

#[test]
fn ordinary_prose_with_a_phrase_said_often_is_not_a_loop() {
    let mut prose = String::new();
    for day in [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ] {
        let _ = write!(
            prose,
            "On {day} the build passed on every platform we test. On {day} the build passed \
             again after the cache was cleared, and the numbers held. "
        );
    }
    assert!(!repeats(&answered_then(&prose), 17));
}

#[test]
fn a_long_reply_keeps_what_it_holds_bounded() {
    let mut looping = Looping::default();
    for line in 0..2_000 {
        assert!(!looping.see(&format!("Line {line} of a long and varied reply.\n")));
    }
    assert!(looping.kept.len() <= KEPT);
    assert!(looping.seen.len() <= KEPT && looping.order.len() <= KEPT);
}
