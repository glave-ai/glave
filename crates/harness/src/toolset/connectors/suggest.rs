//! The connectors of glave's catalog the person may connect and has not, which `tool_search`
//! finds for them to connect: those a search names, whatever connected tools it finds, and,
//! when it finds none, the best by BM25 over each one's name, what it reaches and its tags. The
//! model is told each in a plain line, and the thread records each as a suggestion the person
//! sees (`connector.suggested`); only the person connects one. The catalog is static, so the
//! tool's listing never changes as they are found or connected.

use glave_event::{CallId, ConnectorSuggested, Suggests, Worded as _};
use glave_ports::{ToolOutput, ToolSpec};
use serde_json::json;

use super::{TOOL_SEARCH, rank};
use crate::own_tools::Connectable;

/// The most connectors one search suggests.
const MOST_SUGGESTED: usize = 3;

/// What the search's description says of them where each connector is one tool.
pub(super) const ALSO: &str = " It also finds services the person has not connected yet, which \
     they can connect.";

/// What the model is told after the line of each one suggested.
const ONLY_THEY: &str = "Only the person can connect a service, and they are shown how. Tell \
     them what it would let you do; once they connect it, its tools come with their next message.";

/// The search, as the model is told it where connected tools are listed in full: of them, and
/// of the services the person may connect.
pub(super) fn spec() -> ToolSpec {
    ToolSpec {
        name: TOOL_SEARCH.to_owned(),
        description: "Find a service's tools by what they do, among the services the person \
             connected and those they have not connected yet, which they can connect. Give a \
             few words for what you need, or a service's name."
            .to_owned(),
        parameters: json!({"type": "object", "properties": {
            "query": {"type": "string", "description": "Words for what you need, or a service's name"}
        }, "required": ["query"]}),
    }
}

/// What a search found: what the model is told, and the connectors it suggests the person
/// connect.
#[derive(Debug)]
pub(crate) struct Searched {
    pub(crate) output: ToolOutput,
    pub(crate) suggested: Vec<Connectable>,
}

impl Searched {
    /// `output`, suggesting none.
    pub(super) fn only(output: ToolOutput) -> Self {
        Self {
            output,
            suggested: Vec::new(),
        }
    }

    /// `said`, what was found, then a line for each of `suggested`.
    pub(super) fn with(said: Option<String>, suggested: &[&Connectable]) -> Self {
        let lines = suggested.iter().map(|it| {
            format!(
                "{} isn't connected; the person can connect it. It reaches: {}",
                it.name,
                it.description.trim()
            )
        });
        let mut text: Vec<String> = said.into_iter().chain(lines).collect();
        if !suggested.is_empty() {
            text.push(ONLY_THEY.to_owned());
        }
        Self {
            output: ToolOutput::text(false, text.join("\n")),
            suggested: suggested.iter().map(|it| (*it).clone()).collect(),
        }
    }
}

/// Which of `connectable` a search for `query` suggests, at most [`MOST_SUGGESTED`]: those it
/// names, by a word that is a connector's id or its name, whatever connected tools it found
/// (`found_any`); then, where it found none, the best of the rest by BM25. A word of two letters
/// or fewer says nothing of a service (`to` would find to-do lists), and a name is one word
/// however it is cased, so `post` never finds PostHog. One named whose name is also a plain word
/// (Make, Close, Square) is suggested only where the query's other words, if it has any, find
/// what it reaches or its tags too: "make a linear issue" names Linear, not Make.
pub(super) fn matching<'a>(
    connectable: &'a [Connectable],
    query: &str,
    found_any: bool,
) -> Vec<&'a Connectable> {
    let asked: Vec<String> = (super::words_of(query).into_iter())
        .filter(|word| word.chars().count() > 2)
        .collect();
    let calls = |it: &Connectable, word: &str| it.id == word || it.name.to_lowercase() == word;
    let named: Vec<&Connectable> = (connectable.iter())
        .filter(|it| asked.iter().any(|word| calls(it, word)))
        .collect();
    let fits = |it: &Connectable| {
        let about = rank::words(&format!("{} {}", it.description, it.tags.join(" ")));
        let mut others = asked.iter().filter(|word| !calls(it, word)).peekable();
        others.peek().is_none()
            || others.any(|word| rank::words(word).iter().any(|w| about.contains(w)))
    };
    let mut chosen: Vec<&Connectable> = named.iter().copied().filter(|it| fits(it)).collect();
    if !found_any {
        let specs: Vec<ToolSpec> = (connectable.iter())
            .map(|it| ToolSpec {
                name: it.id.clone(),
                description: format!(
                    "{} {} {}",
                    it.name.to_lowercase(),
                    it.description,
                    it.tags.join(" ")
                ),
                parameters: json!({}),
            })
            .collect();
        let specs: Vec<&ToolSpec> = specs.iter().collect();
        for found in rank::matching(&specs, &asked.join(" ")) {
            // One named that the query's other words do not find stays out, however it ranks.
            let unnamed = connectable
                .iter()
                .find(|it| it.id == found.name)
                .filter(|it| !named.iter().any(|known| known.id == it.id));
            chosen.extend(unnamed);
        }
    }
    chosen.truncate(MOST_SUGGESTED);
    chosen
}

/// The record that the search `call` suggests the person connect `it`.
pub(crate) fn suggestion(call: &CallId, it: &Connectable) -> ConnectorSuggested {
    let name = it.name.clone();
    let suggests = if it.signs_in {
        Suggests::SignIn { name }
    } else {
        Suggests::Connect { name }
    };
    ConnectorSuggested {
        call_id: call.clone(),
        connector: it.id.clone(),
        told: suggests.english(),
        suggests,
    }
}
