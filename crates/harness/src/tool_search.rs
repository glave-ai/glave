//! Connected tools kept out of the model's list while they would crowd its context: past a tenth
//! of the window, their definitions wait behind `tool_search`, which lists each by name and a
//! line and loads the ones the model finds, for the rest of the thread (`tools.loaded`).
//!
//! The list stays stable for the prompt cache: a loaded tool only ever goes on its end, in the
//! order it was loaded. Tools come first in a provider's prefix, so a load misses the cache once,
//! for the whole conversation after them, and every request between loads can hit. Codex and
//! Claude Code avoid even that with their providers' own deferred tools, which the one seam
//! every model shares, an OpenAI-compatible API, doesn't have.

use std::collections::HashMap;
use std::fmt::Write as _;

use glave_event::{Arguments, Body, Event, ToolsLoaded};
use glave_ports::{
    BlobStore, EventLog, ModelProvider, ToolCall, ToolOutput, ToolSpec, describe_call,
};
use serde_json::{Value, json};

use crate::approval::Approver;
use crate::compaction::tools_tokens;
use crate::harness::{Harness, HarnessError, Thread};
use crate::toolset::ToolSet;

/// The tool's name.
pub(crate) const TOOL_SEARCH: &str = "tool_search";

/// The share of the window connected tools may take before they wait behind the search: one
/// part in this many, as Claude Code's `auto` mode has it.
const SHARE: u64 = 10;

/// The most tools one search loads, since every one stays in each request after it.
const MOST_FOUND: usize = 5;

/// The most characters a tool's line in the listing has.
const LINE: usize = 100;

/// BM25's usual constants: how soon more of a word stops counting, and how much a long
/// description is marked down for its length.
const K1: f64 = 1.2;
const B: f64 = 0.75;

/// Whether connected tools `specs` take more of a window of `window_tokens` than they may.
pub(crate) fn crowds(specs: &[ToolSpec], window_tokens: u64) -> bool {
    tools_tokens(specs) > window_tokens / SHARE
}

/// How many characters the listing may take in a window of `window_tokens`: half of what the
/// tools themselves may, so the listing never crowds the window in their place.
pub(crate) fn listing_room(window_tokens: u64) -> usize {
    let tokens = window_tokens / SHARE / 2;
    usize::try_from(crate::compaction::chars_of_tokens(tokens)).unwrap_or(usize::MAX)
}

/// The tool, listing `specs` in order until `room` characters are taken. It never changes as
/// tools load, so a load only adds to the list after it.
pub(crate) fn spec(specs: &[&ToolSpec], room: usize) -> ToolSpec {
    let mut listed = String::new();
    let mut shown = 0;
    for tool in specs {
        let line = format!("- {}: {}\n", tool.name, one_line(&tool.description));
        if listed.len().saturating_add(line.len()) > room {
            break;
        }
        listed.push_str(&line);
        shown += 1;
    }
    let more = specs.len().saturating_sub(shown);
    if more > 0 {
        let _ = writeln!(listed, "- and {more} more, which a search finds too");
    }
    ToolSpec {
        name: TOOL_SEARCH.to_owned(),
        description: format!(
            "Find and load connected tools. These are not in your tool list until you load \
             them, so when a task needs one, search first: by what it does, or by its exact \
             name (several, separated by spaces). What you load you can then call like any \
             other tool, for the rest of this conversation.\n\nConnected tools:\n{listed}"
        ),
        parameters: json!({"type": "object", "properties": {
            "query": {"type": "string", "description": "Words for what the tool does, or exact tool names"}
        }, "required": ["query"]}),
    }
}

/// What a call of a tool that is not loaded gets instead of running, since the model has not
/// been sent its parameters. A call in the same response as the search that loads it does run,
/// asking as any other call does.
pub(crate) fn not_loaded(tool: &str) -> ToolOutput {
    ToolOutput::text(
        true,
        format!("{tool} is not loaded yet. Call {TOOL_SEARCH} with its name, then call it."),
    )
}

/// A `tool_search` call in a line, for the person: what was searched for.
pub(crate) fn describe(arguments: &Arguments) -> String {
    query(arguments).map_or_else(
        || describe_call(TOOL_SEARCH, arguments),
        |query| format!("{TOOL_SEARCH} {query}"),
    )
}

/// The tools `tools.loaded` loaded in `events`, oldest first.
pub(crate) fn loaded_in(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match &event.body {
            Body::ToolsLoaded(loaded) => Some(loaded.tools.iter().cloned()),
            _ => None,
        })
        .flatten()
        .collect()
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Runs a `tool_search` call: what it finds is on offer from the next model call on, and
    /// the thread's for good, recorded first, as a plan is, so a resumed thread has them.
    pub(crate) async fn search_tools(
        &self,
        thread: &mut Thread,
        call: &ToolCall,
        tools: &mut ToolSet,
    ) -> Result<ToolOutput, HarnessError> {
        let Some(query) = query(&call.arguments) else {
            let text = "Say what to search for, in query.";
            return Ok(ToolOutput::text(true, text));
        };
        let found = tools.deferred_matching(query);
        if found.is_empty() {
            let text = format!("No connected tool matches {query:?}. Try other words.");
            return Ok(ToolOutput::text(false, text));
        }
        let names: Vec<String> = found.iter().map(|tool| tool.name.clone()).collect();
        let new = tools.load(&names);
        if !new.is_empty() {
            let loaded = ToolsLoaded { tools: new };
            self.record(thread, &loaded).await?;
            // Glave's own record: call ids do not count it, so the model is sent the same.
            thread.unnumbered = thread.unnumbered.saturating_add(1);
            thread.loaded.extend(loaded.tools);
        }
        let lines: Vec<String> = found
            .iter()
            .map(|tool| format!("- {}: {}", tool.name, one_line(&tool.description)))
            .collect();
        let text = format!("Yours to call now:\n{}", lines.join("\n"));
        Ok(ToolOutput::text(false, text))
    }
}

/// What a call searches for, when it says.
fn query(arguments: &Arguments) -> Option<&str> {
    let Arguments::Json(map) = arguments else {
        return None;
    };
    let query = map.get("query")?.as_str()?.trim();
    (!query.is_empty()).then_some(query)
}

/// Which of `tools` `query` finds: the ones it names when it is nothing but tool names, else
/// the best by BM25, at most [`MOST_FOUND`] and the same way every time, so a search made again
/// loads nothing else. What it leaves out, its name finds, which the listing gives.
pub(crate) fn matching<'a>(tools: &[&'a ToolSpec], query: &str) -> Vec<&'a ToolSpec> {
    let asked: Vec<&str> = query
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|word| !word.is_empty())
        .collect();
    let named: Vec<&'a ToolSpec> = asked
        .iter()
        .filter_map(|word| {
            let found = tools.iter().find(|t| t.name.eq_ignore_ascii_case(word))?;
            Some(*found)
        })
        .collect();
    if named.len() == asked.len() {
        return named.into_iter().take(MOST_FOUND).collect();
    }
    ranked(tools, query)
        .into_iter()
        .take(MOST_FOUND)
        .filter_map(|index| tools.get(index).copied())
        .collect()
}

/// The words of a tool's parameters: their names and descriptions, not the schema's own.
fn schema_text(schema: &Value, text: &mut String) {
    match schema {
        Value::Object(map) => {
            for (key, value) in map {
                match (key.as_str(), value) {
                    ("description", Value::String(said)) => {
                        text.push_str(said);
                        text.push(' ');
                    }
                    ("properties", Value::Object(properties)) => {
                        for (name, property) in properties {
                            text.push_str(name);
                            text.push(' ');
                            schema_text(property, text);
                        }
                    }
                    _ => schema_text(value, text),
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|item| schema_text(item, text)),
        _ => {}
    }
}

/// The indexes of `tools` that `query` is about, best first, by BM25 over each tool's name, which
/// counts twice, its description and its parameters; a tie stays in `tools`' order. Sentences
/// are full of words most tools have, so where a query has rarer words, a tool must have one of
/// them, and a word no tool has counts as rarer: finding nothing, the model can search again,
/// but what a search finds stays in every request after. For that too, what scores under half
/// the best is left out.
fn ranked(tools: &[&ToolSpec], query: &str) -> Vec<usize> {
    let documents: Vec<HashMap<String, u32>> = tools
        .iter()
        .map(|tool| {
            let mut text = format!("{name} {name} {} ", tool.description, name = tool.name);
            schema_text(&tool.parameters, &mut text);
            counted(&text)
        })
        .collect();
    let lengths: Vec<f64> = documents
        .iter()
        .map(|document| f64::from(document.values().sum::<u32>()))
        .collect();
    let count = f64::from(u32::try_from(tools.len()).unwrap_or(u32::MAX));
    let average = (lengths.iter().sum::<f64>() / count).max(1.0);
    let having = |word: &String| {
        let found = documents.iter().filter(|d| d.contains_key(word)).count();
        f64::from(u32::try_from(found).unwrap_or(u32::MAX))
    };
    let mut asked = words(query);
    asked.sort_unstable();
    asked.dedup();
    let rarer: Vec<&String> = asked.iter().filter(|w| having(w) * 2.0 <= count).collect();
    let mut scored: Vec<(usize, f64)> = documents
        .iter()
        .zip(&lengths)
        .enumerate()
        .filter(|(_, (document, _))| {
            rarer.is_empty() || rarer.iter().any(|word| document.contains_key(*word))
        })
        .map(|(index, (document, length))| {
            let score = asked
                .iter()
                .filter_map(|word| Some((word, f64::from(*document.get(word)?))))
                .map(|(word, times)| {
                    let having = having(word);
                    let rarity = (1.0 + (count - having + 0.5) / (having + 0.5)).ln();
                    let fill = times + K1 * (1.0 - B + B * length / average);
                    rarity * times * (K1 + 1.0) / fill
                })
                .sum();
            (index, score)
        })
        .filter(|(_, score)| *score > 0.0)
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    let best = scored.first().map_or(0.0, |(_, score)| *score);
    scored
        .into_iter()
        .filter(|(_, score)| *score >= best / 2.0)
        .map(|(index, _)| index)
        .collect()
}

/// How many times each of `text`'s words is in it.
fn counted(text: &str) -> HashMap<String, u32> {
    let mut counts = HashMap::new();
    for word in words(text) {
        let times: &mut u32 = counts.entry(word).or_default();
        *times = times.saturating_add(1);
    }
    counts
}

/// `text`'s words, lowercase and without a plural's `s`, parted where it is neither a letter nor
/// a digit and inside a name in camelCase: `listOpenIssues` is `list`, `open`, `issue`.
fn words(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut word = String::new();
    let mut lower_before = false;
    for c in text.chars() {
        let boundary = !c.is_alphanumeric() || (c.is_uppercase() && lower_before);
        if boundary && !word.is_empty() {
            found.push(singular(std::mem::take(&mut word)));
        }
        lower_before = c.is_lowercase();
        if c.is_alphanumeric() {
            word.extend(c.to_lowercase());
        }
    }
    if !word.is_empty() {
        found.push(singular(word));
    }
    found
}

/// `word` without a plural's `s`, however rough: the same word gets the same answer, which is
/// all a search needs.
fn singular(word: String) -> String {
    match word.strip_suffix('s') {
        Some(stem) if stem.chars().count() > 2 && !stem.ends_with('s') => stem.to_owned(),
        _ => word,
    }
}

/// A tool's description in a line: its first sentence, at most [`LINE`] characters.
fn one_line(description: &str) -> String {
    let line = description.trim().lines().next().unwrap_or_default();
    let sentence = line.split_once(". ").map_or(line, |(first, _)| first);
    let sentence = sentence.trim_end_matches('.');
    if sentence.chars().count() > LINE {
        let cut: String = sentence.chars().take(LINE).collect();
        return format!("{cut}…");
    }
    sentence.to_owned()
}

#[cfg(test)]
mod tests;
