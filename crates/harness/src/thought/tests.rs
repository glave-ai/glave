use std::time::Duration;

use glave_event::{Part, Thought};
use glave_ports::Delta;
use jiff::Timestamp;

use super::{Thinking, headline};
use crate::turn::Live;

fn at(millis: i64) -> Timestamp {
    Timestamp::from_millisecond(millis).unwrap_or(Timestamp::UNIX_EPOCH)
}

fn reasoning(text: &str) -> Part {
    Part::Reasoning {
        text: Some(text.to_owned()),
        opaque: None,
    }
}

#[test]
fn the_headline_is_the_last_whole_sentence() {
    assert_eq!(
        headline("The user wants it stopped. Checking the port"),
        Some("The user wants it stopped.".to_owned())
    );
    assert_eq!(
        headline("The user wants it stopped. Checking the port 5173."),
        Some("Checking the port 5173.".to_owned())
    );
    // A stop inside a word or a number ends nothing.
    assert_eq!(headline("Version 3.5 of vite.config.ts is here"), None);
    // A line that ended is whole without its stop; marks and runs of spaces go.
    assert_eq!(
        headline("- list the   `src` folder\nthen"),
        Some("list the src folder".to_owned())
    );
    assert_eq!(headline("Fertig！ 次は？"), Some("次は？".to_owned()));
    assert_eq!(headline(""), None);
    assert_eq!(headline("   \n\n"), None);
}

#[test]
fn a_bold_title_on_its_own_line_is_the_headline() {
    let text = "**Checking the port**\n\nThe server runs. I'll stop it.\n\n**Removing the files**\n\nBy name.";
    assert_eq!(headline(text), Some("Removing the files".to_owned()));
    // A title still being written is none yet; bold inside a sentence is no title.
    assert_eq!(
        headline("**Checking the port**\n\nDone.\n\n**Remov"),
        Some("Checking the port".to_owned())
    );
    assert_eq!(
        headline("This is **important** to do."),
        Some("This is important to do.".to_owned())
    );
}

#[test]
fn a_long_headline_is_cut() {
    let long = format!("{}.", "word ".repeat(100));
    let cut = headline(&long).unwrap_or_default();
    assert_eq!(cut.chars().count(), 200);
    assert!(cut.ends_with('…'));
}

#[test]
fn streaming_reasoning_tells_each_new_headline_and_how_long_it_took() {
    let mut thinking = Thinking::default();
    let mut shown = Vec::new();
    let mut live = |update: Live| shown.push(update);
    thinking.see(
        Delta::Reasoning("The user wants".to_owned()),
        at(1_000),
        &mut live,
    );
    thinking.see(
        Delta::Reasoning(" it stopped. Che".to_owned()),
        at(1_500),
        &mut live,
    );
    thinking.see(Delta::Reasoning("cking. ".to_owned()), at(2_000), &mut live);
    // The same headline again tells nothing.
    thinking.see(Delta::Reasoning(" ".to_owned()), at(2_100), &mut live);
    thinking.see(Delta::Text("Stopped".to_owned()), at(5_250), &mut live);
    thinking.see(Delta::Text(".".to_owned()), at(6_000), &mut live);
    thinking.end(at(7_000), &mut live);
    assert_eq!(
        shown,
        vec![
            Live::Reasoning("The user wants".to_owned()),
            Live::Reasoning(" it stopped. Che".to_owned()),
            Live::Thinking("The user wants it stopped.".to_owned()),
            Live::Reasoning("cking. ".to_owned()),
            Live::Thinking("Checking.".to_owned()),
            Live::Reasoning(" ".to_owned()),
            Live::Thought(Duration::from_millis(4_250)),
            Live::Text("Stopped".to_owned()),
            Live::Text(".".to_owned()),
        ]
    );
    let parts = [reasoning("The user wants it stopped. Checking. ")];
    assert_eq!(
        thinking.thought(&parts),
        Some(Thought {
            headline: Some("Checking.".to_owned()),
            ms: Some(4_250),
        })
    );
}

#[test]
fn reasoning_ends_with_the_call_when_nothing_else_is_written() {
    let mut thinking = Thinking::default();
    let mut shown = Vec::new();
    let mut live = |update: Live| shown.push(update);
    thinking.see(
        Delta::Reasoning("Run the tests.".to_owned()),
        at(0),
        &mut live,
    );
    thinking.end(at(3_000), &mut live);
    thinking.end(at(9_000), &mut live);
    assert_eq!(shown.last(), Some(&Live::Thought(Duration::from_secs(3))));
    assert_eq!(
        shown
            .iter()
            .filter(|update| matches!(update, Live::Thought(_)))
            .count(),
        1
    );
}

#[test]
fn a_message_without_readable_reasoning_has_no_thought() {
    let thinking = Thinking::default();
    assert_eq!(thinking.thought(&[]), None);
    assert_eq!(thinking.thought(&[reasoning("  ")]), None);
    // Reasoning that did not stream has a headline and no time.
    assert_eq!(
        thinking.thought(&[reasoning("Done.")]),
        Some(Thought {
            headline: Some("Done.".to_owned()),
            ms: None,
        })
    );
    // Nothing else alone: text and no time are no thought.
    let mut nothing = Thinking::default();
    let mut shown = Vec::new();
    nothing.end(at(1), &mut |update| shown.push(update));
    assert_eq!(shown, Vec::<Live>::new());
}
