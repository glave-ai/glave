//! What tidy-up asks the model and how its answer is read: the memories as a numbered list
//! (never their ids, which a cheap model copies badly), proposals back as JSON.

use std::fmt::Write as _;

use glave_event::{At, Part, StopReason};
use glave_ports::memories::MemoryError;
use glave_ports::{Message, ModelProvider, ModelRequest, Purpose, ToolChoice};
use serde::Deserialize;

use super::Tidier;
use crate::book::Entry;

/// The longest answer tidy-up reads, in tokens: room for a reasoning model to think first.
const ANSWER_TOKENS: u32 = 8_000;

const SYSTEM: &str = "You tidy the memories an assistant keeps about a person and their work, \
     so they stay few and true. You only merge and settle what is listed: never add a fact, \
     never drop a detail the person asked to keep, never reword a memory that needs no change.\n\
     \n\
     - merge: memories that say the same thing (duplicates, rewordings), or, only when asked \
     to compact, closely related memories about one subject. Give the merged text: one short \
     statement, at most 500 characters, using the memories' own words and every detail they \
     hold.\n\
     - conflict: two memories that say different things about the same fact (such as where \
     the project deploys, or which tool it uses). The person said both, at different times, \
     so the later one is how things are now: clear is true. clear is false only when both \
     could hold at once (they are about different contexts) or you cannot tell they are \
     about the same fact.\n\
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

/// The answer's object. It names `merge` or `conflict`, or both: an object with neither (one of
/// its proposals, or a draft cut off around one) is not an answer, so it is never read as
/// "nothing to do".
#[derive(Deserialize)]
struct Answer {
    merge: Option<Vec<Merge>>,
    conflict: Option<Vec<Conflict>>,
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

/// When, to the minute.
fn minute(at: At) -> String {
    at.to_string()
        .chars()
        .take(16)
        .collect::<String>()
        .replace('T', " ")
}

/// The request: the candidates as a numbered list, oldest first, with their times: when each
/// was saved and, when later, last said (`Entry::stated`), the time a contradiction is settled
/// by, never tidy-up's own merges.
fn request(model: &str, candidates: &[Entry], compact: bool) -> ModelRequest {
    let mut text = String::from("The memories, oldest first:\n");
    for (number, entry) in (1_usize..).zip(candidates) {
        let Some((_, current)) = entry.current() else {
            continue;
        };
        let saved = entry.versions.first().map(|version| version.at);
        let _ = write!(
            text,
            "\n{number}. (saved {}",
            saved.map_or_else(String::new, minute)
        );
        if let Some(said) = entry.stated().filter(|said| Some(*said) != saved) {
            let _ = write!(text, ", changed {}", minute(said));
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
        files: Vec::new(),
        effort: None,
    }
}

/// What a second ask adds when the first answer held no proposals in the shape asked for.
const REMINDER: &str = "Answer with the JSON object only, in the shape given: {\"merge\": [...], \
     \"conflict\": [...]}. Use empty lists when there is nothing to do.";

/// Asks the model what to tidy, once more with a terse reminder when the first answer cannot
/// be read. The proposals, or why there are none.
pub(super) async fn ask<P: ModelProvider>(
    tidier: &Tidier<P>,
    candidates: &[Entry],
    compact: bool,
) -> Result<Result<Vec<Proposal>, String>, MemoryError> {
    let mut request = request(&tidier.model, candidates, compact);
    let (first, cut) = answer(tidier, &request).await?;
    let why = match read(&first, cut, candidates.len()) {
        Ok(proposals) => return Ok(Ok(proposals)),
        Err(why) => why,
    };
    request.messages.push(Message::Assistant {
        parts: vec![Part::Text { text: first }],
        tool_calls: Vec::new(),
    });
    request.messages.push(Message::User {
        parts: vec![Part::Text {
            text: REMINDER.to_owned(),
        }],
    });
    let (second, cut) = answer(tidier, &request).await?;
    Ok(
        read(&second, cut, candidates.len())
            .map_err(|again| format!("{why}; asked again: {again}")),
    )
}

/// The proposals in an answer, unless it was `cut` off at its length limit: what it holds then
/// is a draft, never the whole answer.
fn read(answer: &str, cut: bool, listed: usize) -> Result<Vec<Proposal>, String> {
    if cut {
        return Err(format!(
            "tidy-up's model was cut off at {ANSWER_TOKENS} tokens"
        ));
    }
    parse(answer, listed)
}

/// The answer's text, or its readable reasoning when it gave no text, as some reasoning models
/// do; and whether it was cut off at its length limit.
async fn answer<P: ModelProvider>(
    tidier: &Tidier<P>,
    request: &ModelRequest,
) -> Result<(String, bool), MemoryError> {
    let completion = tidier
        .provider
        .complete(request, &mut |_| {})
        .await
        .map_err(|error| MemoryError::Storage(format!("tidy-up's model failed: {error}")))?;
    let cut = completion.stop == StopReason::MaxTokens;
    let text: String = completion
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    if text.contains('{') {
        return Ok((text, cut));
    }
    let reasoning: String = completion
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::Reasoning {
                text: Some(text), ..
            } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    let shown = if reasoning.contains('{') {
        reasoning
    } else {
        text
    };
    Ok((shown, cut))
}

/// The proposals in `answer`: the JSON object in it, however it is wrapped. Numbers outside the
/// list are left for the checks to refuse one by one.
pub(super) fn parse(answer: &str, listed: usize) -> Result<Vec<Proposal>, String> {
    let mut wrong = None;
    let mut found = None;
    // The first object that reads as an answer, wherever it starts: after prose, inside a
    // code fence, before more text.
    for (start, _) in answer.match_indices('{') {
        let rest = answer.get(start..).unwrap_or_default();
        match serde_json::Deserializer::from_str(rest)
            .into_iter::<Answer>()
            .next()
        {
            Some(Ok(parsed)) if parsed.merge.is_some() || parsed.conflict.is_some() => {
                found = Some(parsed);
                break;
            }
            Some(Err(error)) if wrong.is_none() => wrong = Some(error.to_string()),
            _ => {}
        }
    }
    let Some(parsed) = found else {
        return Err(wrong.map_or_else(
            || format!("tidy-up's model gave no JSON among {listed} memories"),
            |error| format!("tidy-up's model answered in the wrong shape: {error}"),
        ));
    };
    let merges = parsed
        .merge
        .unwrap_or_default()
        .into_iter()
        .map(|merge| Proposal::Merge {
            memories: merge.memories,
            text: merge.text,
        });
    let conflicts = parsed
        .conflict
        .unwrap_or_default()
        .into_iter()
        .map(|conflict| Proposal::Conflict {
            memories: conflict.memories,
            clear: conflict.clear,
        });
    // Contradictions first: a memory a newer one replaces is forgotten, not merged.
    Ok(conflicts.chain(merges).collect())
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
        assert_eq!(parse("{\"merge\": []}", 2).unwrap(), []);
        assert!(parse("{}", 2).is_err(), "names neither list");
        // Cut off in its second proposal: the first, complete on its own, is not the answer.
        let cut = "{\"merge\": [{\"memories\": [1, 2], \"text\": \"Use tabs\"}, {\"memories\": [3";
        assert!(parse(cut, 4).is_err(), "{:?}", parse(cut, 4));
        assert!(parse("nothing to do", 2).is_err());
        assert!(parse("{\"merge\": 3}", 2).is_err());
        let prose = "Looking at {these} memories, 1 and 2 match.\n{\"merge\": [], \"conflict\": \
                     [{\"memories\": [1, 2], \"clear\": true}]}\nDone {ok}.";
        assert_eq!(parse(prose, 2).unwrap().len(), 1);
    }
}
