//! The connectors of glave's catalog the person may connect and has not, which `tool_search`
//! finds for them to connect: those a search names, whatever connected tools it finds, and,
//! when it names none and finds no connected tool, the best by BM25 over each one's name, what
//! it reaches and its tags. The model is told each in a plain line, and the thread records each
//! as a suggestion the person sees (`connector.suggested`); only the person connects one. The
//! catalog is static, so the tool's listing never changes as they are found or connected.

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

/// Whether `word`, as the search wrote it, names `it`: a name that is also a plain word (Make,
/// Close, Linear) only written as the catalog spells it (`Make`, `monday` of `monday.com`), as
/// the model writes a service's name; any other in any case, by its id or its name.
fn names(it: &Connectable, word: &str) -> bool {
    if it.plain_word {
        return it.name.split('.').next() == Some(word);
    }
    word.eq_ignore_ascii_case(&it.id) || word.to_lowercase() == it.name.to_lowercase()
}

/// Which of `connectable` a search for `query` suggests, at most [`MOST_SUGGESTED`]: those it
/// names ([`names`]), whatever connected tools it found (`found_any`), and no other beside them;
/// else, where it found none, the best by BM25 over each one's name, what it reaches and its
/// tags, a plain word's name left out, so the verbs `make` and `close` never rank Make or Close.
/// A word of two letters or fewer says nothing of a service (`to` would find to-do lists), and a
/// name is one word however it is cased, so `post` never finds PostHog.
pub(super) fn matching<'a>(
    connectable: &'a [Connectable],
    query: &str,
    found_any: bool,
) -> Vec<&'a Connectable> {
    let written: Vec<&str> = (query.split(|c: char| !c.is_alphanumeric()))
        .filter(|word| word.chars().count() > 2)
        .collect();
    let named: Vec<&Connectable> = (connectable.iter())
        .filter(|it| written.iter().any(|word| names(it, word)))
        .take(MOST_SUGGESTED)
        .collect();
    if found_any || !named.is_empty() {
        return named;
    }
    let specs: Vec<ToolSpec> = (connectable.iter())
        .map(|it| {
            let about = format!("{} {}", it.description, it.tags.join(" "));
            let description = if it.plain_word {
                about
            } else {
                format!("{id} {id} {} {about}", it.name.to_lowercase(), id = it.id)
            };
            ToolSpec {
                name: String::new(),
                description,
                parameters: json!({}),
            }
        })
        .collect();
    let specs: Vec<&ToolSpec> = specs.iter().collect();
    let asked: Vec<String> = (super::words_of(query).into_iter())
        .filter(|word| word.chars().count() > 2)
        .collect();
    (rank::matching(&specs, &asked.join(" ")).into_iter())
        .filter_map(|found| {
            let at = specs.iter().position(|spec| std::ptr::eq(*spec, found))?;
            connectable.get(at)
        })
        .take(MOST_SUGGESTED)
        .collect()
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
