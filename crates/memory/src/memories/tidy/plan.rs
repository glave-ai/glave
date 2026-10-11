//! What tidy-up asks the model and how its answer is read and checked: the memories as a
//! numbered list (never their ids, which a cheap model copies badly), proposals back as JSON,
//! and the check that a merged text keeps every source's facts and adds none.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use glave_event::{At, Part};
use glave_harness::MemoryError;
use glave_ports::{Message, ModelRequest, Purpose, ToolChoice};
use serde::Deserialize;

use super::Tidier;
use crate::book::Entry;
use crate::similar::words;

/// The longest answer tidy-up reads, in tokens.
const ANSWER_TOKENS: u32 = 2_000;
/// Words a merged text may add that no source has: joining words, never facts.
const MOST_NEW_WORDS: usize = 2;
/// The share of each source's words, in percent, a merged text must keep.
const KEEP_PERCENT: usize = 50;
/// Memories one merge may take in.
const MOST_MERGED: usize = 8;

const SYSTEM: &str = "You tidy the memories an assistant keeps about a person and their work, \
     so they stay few and true. You only merge and settle what is listed: never add a fact, \
     never drop a detail the person asked to keep, never reword a memory that needs no change.\n\
     \n\
     - merge: memories that say the same thing (duplicates, rewordings), or, only when asked \
     to compact, closely related memories about one subject. Give the merged text: one short \
     statement, at most 500 characters, using the memories' own words and every detail they \
     hold.\n\
     - conflict: two memories that say different things about the same fact. clear is true \
     when the newer one plainly replaces the older (the fact changed), false when it is \
     unclear which holds or both could be meant.\n\
     \n\
     Leave everything else alone; an empty answer is fine. Answer with JSON only, in this \
     shape:\n\
     {\"merge\": [{\"memories\": [1, 2], \"text\": \"...\"}], \"conflict\": [{\"memories\": [3, \
     4], \"clear\": true}]}";

/// One change the model proposes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Proposal {
    /// These memories become one, with this text.
    Merge { memories: Vec<usize>, text: String },
    /// These two disagree; `clear` when the newer plainly replaces the older.
    Conflict { memories: Vec<usize>, clear: bool },
}

impl Proposal {
    pub(super) fn memories(&self) -> &[usize] {
        match self {
            Self::Merge { memories, .. } | Self::Conflict { memories, .. } => memories,
        }
    }
}

#[derive(Deserialize)]
struct Answer {
    #[serde(default)]
    merge: Vec<Merge>,
    #[serde(default)]
    conflict: Vec<Conflict>,
}

#[derive(Deserialize)]
struct Merge {
    memories: Vec<usize>,
    text: String,
}

#[derive(Deserialize)]
struct Conflict {
    memories: Vec<usize>,
    #[serde(default)]
    clear: bool,
}

fn day(at: At) -> String {
    at.to_string().chars().take(10).collect()
}

/// The request: the candidates as a numbered list, oldest first, with their dates.
fn request(model: &str, candidates: &[Entry], compact: bool) -> ModelRequest {
    let mut text = String::from("The memories, oldest first:\n");
    for (number, entry) in (1_usize..).zip(candidates) {
        let Some((_, current)) = entry.current() else {
            continue;
        };
        let saved = entry
            .versions
            .first()
            .map_or_else(String::new, |v| day(v.at));
        let _ = write!(text, "\n{number}. (saved {saved}");
        if entry.versions.len() > 1 {
            let _ = write!(text, ", changed {}", day(current.at));
        }
        let _ = write!(text, ") {}", current.text);
    }
    text.push_str(if compact {
        "\n\nThese memories are near their size limit: also compact closely related memories \
         into fewer, shorter ones, keeping every detail."
    } else {
        "\n\nMerge only duplicates; do not compact."
    });
    ModelRequest {
        model: model.to_owned(),
        system: SYSTEM.to_owned(),
        messages: vec![Message::User {
            parts: vec![Part::Text { text }],
        }],
        tools: Vec::new(),
        tool_choice: ToolChoice::None,
        max_output_tokens: Some(ANSWER_TOKENS),
        purpose: Purpose::Summary,
        threads: Vec::new(),
    }
}

/// Asks the model what to tidy, and returns its answer's text.
pub(super) async fn ask(
    tidier: &dyn Tidier,
    candidates: &[Entry],
    compact: bool,
) -> Result<String, MemoryError> {
    let completion = tidier
        .complete(request(tidier.model(), candidates, compact))
        .await
        .map_err(|error| MemoryError::Storage(format!("tidy-up's model failed: {error}")))?;
    Ok(completion
        .parts
        .into_iter()
        .filter_map(|part| match part {
            Part::Text { text } => Some(text),
            _ => None,
        })
        .collect::<String>())
}

/// The proposals in `answer`: the JSON object in it, however it is wrapped. Numbers outside the
/// list are left for the checks to refuse one by one.
pub(super) fn parse(answer: &str, listed: usize) -> Result<Vec<Proposal>, String> {
    let (Some(start), Some(end)) = (answer.find('{'), answer.rfind('}')) else {
        return Err(format!(
            "tidy-up's model gave no JSON among {listed} memories"
        ));
    };
    let json = answer.get(start..=end).unwrap_or_default();
    let parsed: Answer = serde_json::from_str(json)
        .map_err(|error| format!("tidy-up's model answered in the wrong shape: {error}"))?;
    let merges = parsed.merge.into_iter().map(|merge| Proposal::Merge {
        memories: merge.memories,
        text: merge.text,
    });
    let conflicts = parsed
        .conflict
        .into_iter()
        .map(|conflict| Proposal::Conflict {
            memories: conflict.memories,
            clear: conflict.clear,
        });
    // Contradictions first: a memory a newer one replaces is forgotten, not merged.
    Ok(conflicts.chain(merges).collect())
}

/// Whether `text` merges `sources` without inventing or losing a fact: it adds at most a couple
/// of joining words (none with a digit), and keeps at least half of each source's words.
pub(super) fn keeps_facts(text: &str, sources: &[String]) -> Result<(), String> {
    if sources.len() > MOST_MERGED {
        return Err(format!("a merge takes at most {MOST_MERGED} memories"));
    }
    let merged = words(text);
    let all: BTreeSet<String> = sources.iter().flat_map(|source| words(source)).collect();
    let new: Vec<&String> = merged.difference(&all).collect();
    if new.len() > MOST_NEW_WORDS {
        return Err(format!("the merged text adds words no memory has: {new:?}"));
    }
    let known: BTreeSet<String> = sources.iter().flat_map(|source| numbers(source)).collect();
    if let Some(number) = numbers(text)
        .into_iter()
        .find(|number| !known.contains(number))
    {
        return Err(format!(
            "the merged text adds {number:?}, which no memory has"
        ));
    }
    for source in sources {
        let theirs = words(source);
        let kept = theirs.intersection(&merged).count();
        if kept.saturating_mul(100) < theirs.len().saturating_mul(KEEP_PERCENT) {
            return Err(format!("the merged text loses most of {source:?}"));
        }
    }
    Ok(())
}

/// Every run of letters and digits in `text` that holds a digit: numbers, versions, names.
fn numbers(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().any(|c| c.is_ascii_digit()))
        .map(str::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_read_however_they_are_wrapped() {
        let answer = "Here you go:\n```json\n{\"merge\": [{\"memories\": [1, 2], \"text\": \"Use \
                      tabs\"}], \"conflict\": [{\"memories\": [3, 4], \"clear\": true}]}\n```";
        assert_eq!(
            parse(answer, 4).unwrap(),
            [
                Proposal::Conflict {
                    memories: vec![3, 4],
                    clear: true
                },
                Proposal::Merge {
                    memories: vec![1, 2],
                    text: "Use tabs".into()
                },
            ]
        );
        assert_eq!(parse("{}", 2).unwrap(), []);
        assert!(parse("nothing to do", 2).is_err());
        assert!(parse("{\"merge\": 3}", 2).is_err());
    }

    #[test]
    fn a_merge_may_join_but_never_invent_or_lose() {
        let sources = [
            "Use tabs, not spaces, in this project's Python".to_owned(),
            "This project's Python code uses tabs for indentation".to_owned(),
        ];
        assert_eq!(
            keeps_facts(
                "Use tabs, not spaces, for indentation in this project's Python code",
                &sources
            ),
            Ok(())
        );
        assert!(keeps_facts("Use tabs in Python and deploy to Render", &sources).is_err());
        assert!(
            keeps_facts(
                "Use tabs, not spaces, width 8, in this project's Python code",
                &sources
            )
            .is_err(),
            "adds a number"
        );
        assert!(
            keeps_facts("Python", &sources).is_err(),
            "loses nearly everything"
        );
    }
}
