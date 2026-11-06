//! Connected tools, when listing them all would crowd the model's list: each connector is then
//! one tool, `mcp__<connector>`, that calls any of its tools by name, and `tool_search` finds
//! them and shows their parameters (#247). The list is part of a thread's prefix, so it never
//! changes as tools are found: a search's result holds what it found, at the end of the
//! conversation, and the prompt cache keeps matching. Every call names the real tool by the
//! time it is asked about, judged and hooked, and in its summary, and its arguments are checked
//! against the real tool's parameters first, since the model may not have seen them.

mod fits;
mod rank;

use std::collections::BTreeSet;
use std::fmt::Write as _;

use glave_event::Arguments;
use glave_ports::{ToolCall, ToolOutput, ToolSpec};
use serde_json::{Map, Value, json};

use super::ToolSet;

/// The search's name.
pub(crate) const TOOL_SEARCH: &str = "tool_search";

/// The share of the window connected tools may take listed in full: one part in this many, as
/// Claude Code's default has it.
const SHARE: u64 = 10;

/// The most tools listed in full, every one counted: Codex's threshold, below the 128 tools
/// OpenAI's and Azure's APIs accept.
const MOST_LISTED: usize = 100;

/// The most of a connector's tools its own tool names, each with a line.
const SAMPLED: usize = 20;

/// The most characters of a tool's line, and of what a connector says of itself.
const LINE: usize = 100;
const ABOUT: usize = 300;

/// The most characters of a found tool's description a search shows.
const DESCRIBED: usize = 1_000;

/// Characters to a token, as the window is estimated everywhere else.
const CHARS_PER_TOKEN: u64 = 4;

/// What a call of the model's comes to before it runs.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Route {
    /// Run this call: the model's own, or the connected tool it stands for.
    Run(ToolCall),
    /// Search the connected tools for `query`, only `connector`'s when it says.
    Search {
        query: Option<String>,
        connector: Option<String>,
    },
    /// Run nothing: this is the call's result.
    Refused(ToolOutput),
}

/// The connector a connected tool called `name` belongs to: `github` of
/// `mcp__github__issue_read`.
pub(crate) fn connector_of(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("mcp__")?;
    let (connector, tool) = rest.split_once("__")?;
    (!connector.is_empty() && !tool.is_empty()).then_some(connector)
}

/// The tool that calls `connector`'s tools.
pub(crate) fn wrapper(connector: &str) -> String {
    format!("mcp__{connector}")
}

/// The connector whose tool `name` is, when it is one: `github` of `mcp__github`.
pub(crate) fn connector_called(name: &str) -> Option<&str> {
    let connector = name.strip_prefix("mcp__")?;
    (!connector.is_empty() && !connector.contains("__")).then_some(connector)
}

/// A connected tool's name within its connector: `issue_read` of `mcp__github__issue_read`.
fn short_of(name: &str) -> &str {
    connector_of(name)
        .and_then(|connector| name.get(wrapper(connector).len().saturating_add(2)..))
        .unwrap_or(name)
}

/// What a call of `name`, listed but not on offer now, is told.
pub(crate) fn not_available(name: &str) -> String {
    format!(
        "{name:?} is not available now: its connection was turned off, or the service behind it \
         is not answering. Nothing ran. Go on without it, or tell the person."
    )
}

/// `text` in a line: its first sentence, at most `most` characters.
fn one_line(text: &str, most: usize) -> String {
    let line = text.trim().lines().next().unwrap_or_default();
    let sentence = line.split_once(". ").map_or(line, |(first, _)| first);
    let sentence = sentence.trim().trim_end_matches('.');
    if sentence.chars().count() > most {
        let cut: String = sentence.chars().take(most).collect();
        return format!("{cut}…");
    }
    sentence.to_owned()
}

/// The search, as the model is told it.
fn search_spec() -> ToolSpec {
    ToolSpec {
        name: TOOL_SEARCH.to_owned(),
        description: "Find connected tools by what they do, and see their parameters. Each \
             connector is one tool, mcp__<connector>, which lists some of its tools and calls \
             any of them by name: search for the rest, or for one's parameters, before calling \
             it. Give a few words for what the tool does, or its exact name; name a connector to \
             search only its tools."
            .to_owned(),
        parameters: json!({"type": "object", "properties": {
            "query": {"type": "string", "description": "Words for what the tool does, or its exact name"},
            "connector": {"type": "string", "description": "Search only this connector's tools: the <connector> of mcp__<connector>"}
        }, "required": ["query"]}),
    }
}

/// The tool that calls `connector`'s `tools`, listing the first [`SAMPLED`] of them with a line
/// each, after what the connector says of itself, `about`.
fn wrapper_spec(connector: &str, tools: &[&ToolSpec], about: Option<&String>) -> ToolSpec {
    let mut description = format!(
        "Calls a tool of the {connector} connector ({} in all): its name in `tool`, its \
         arguments in `arguments`. Find a tool and its parameters first with {TOOL_SEARCH} \
         (connector \"{connector}\").",
        tools.len()
    );
    if let Some(about) = about {
        let _ = write!(description, "\n{connector}: {about}");
    }
    description.push_str("\nIts tools include:");
    for tool in tools.iter().take(SAMPLED) {
        let name = short_of(&tool.name);
        let _ = write!(
            description,
            "\n- {name}: {}",
            one_line(&tool.description, LINE)
        );
    }
    let more = tools.len().saturating_sub(SAMPLED);
    if more > 0 {
        let _ = write!(
            description,
            "\n- and {more} more, which {TOOL_SEARCH} finds"
        );
    }
    ToolSpec {
        name: wrapper(connector),
        description,
        parameters: json!({"type": "object", "properties": {
            "tool": {"type": "string", "description": "The tool's name, as listed or as tool_search shows it"},
            "arguments": {"type": "object", "description": "The tool's arguments, as its parameters say", "additionalProperties": true}
        }, "required": ["tool"]}),
    }
}

/// What a search found: each tool's connector, name, description and parameters, and how to
/// call one.
fn found_text(found: &[&ToolSpec]) -> String {
    let mut example = None;
    let tools: Vec<Value> = found
        .iter()
        .filter_map(|tool| {
            let connector = connector_of(&tool.name)?;
            let name = short_of(&tool.name);
            example.get_or_insert_with(|| (wrapper(connector), name.to_owned()));
            let description: String = tool.description.chars().take(DESCRIBED).collect();
            Some(json!({
                "connector": wrapper(connector),
                "tool": name,
                "description": description,
                "parameters": tool.parameters,
            }))
        })
        .collect();
    let (call, name) = example.unwrap_or_default();
    format!(
        "Call one through its connector's tool, with its name and arguments: {call} with \
         {{\"tool\": \"{name}\", \"arguments\": {{…}}}}.\n{}",
        Value::Array(tools)
    )
}

/// What a call's JSON arguments are, when they are JSON.
fn map_of(arguments: &Arguments) -> Option<&Map<String, Value>> {
    match arguments {
        Arguments::Json(map) => Some(map),
        Arguments::Malformed(_) => None,
    }
}

/// The string `key` holds in `map`, trimmed, when it holds a string that isn't empty.
fn text_of<'a>(map: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    let text = map.get(key)?.as_str()?.trim();
    (!text.is_empty()).then_some(text)
}

/// A search of `connector`'s tools, or of them all, for what `map` asks: its `query`, or the
/// `query` of its `arguments`, as a model may send it through a connector's tool.
fn search_of(map: Option<&Map<String, Value>>, connector: Option<&str>) -> Route {
    let inner = map.and_then(|map| map.get("arguments")?.as_object());
    let query = (map.and_then(|map| text_of(map, "query")))
        .or_else(|| inner.and_then(|inner| text_of(inner, "query")));
    let named = map.and_then(|map| text_of(map, "connector"));
    Route::Search {
        query: query.map(str::to_owned),
        connector: connector.or(named).map(str::to_owned),
    }
}

impl ToolSet {
    /// Says, in a line, what the connected tools of one source, those of the catalog from
    /// `first` on, say of themselves, `instructions`, when they are all one connector's.
    pub(super) fn about_from(&mut self, first: usize, instructions: Option<String>) {
        let listed = self.catalog.get(first..).unwrap_or_default();
        let mut connectors = listed.iter().filter_map(|name| connector_of(name));
        let (Some(connector), Some(instructions)) = (connectors.next(), instructions) else {
            return;
        };
        if connectors.all(|other| other == connector) {
            let line = one_line(&instructions, ABOUT);
            if !line.is_empty() {
                let connector = connector.to_owned();
                self.about.insert(connector, line);
            }
        }
    }

    /// The connected tools on offer, by name.
    pub(crate) fn connected_names(&self) -> BTreeSet<String> {
        self.catalog.iter().cloned().collect()
    }

    /// Whether the tool called `name` is a connected one on offer.
    fn is_connected(&self, name: &str) -> bool {
        self.get(name)
            .is_some_and(|(index, _)| self.connected.get(index) == Some(&true))
    }

    /// Lists each connector as one tool, and `tool_search` before them, where the connected
    /// tools' definitions would take more than a tenth of a window of `window_tokens`, or there
    /// would be more than [`MOST_LISTED`] tools in all; else leaves every tool listed in full.
    pub(crate) fn frame(&mut self, window_tokens: u64) {
        let connected: Vec<&ToolSpec> = (self.specs.iter())
            .filter(|spec| self.is_connected(&spec.name))
            .collect();
        let chars = serde_json::to_string(&connected).map_or(0, |json| json.len());
        let tokens = u64::try_from(chars).unwrap_or(u64::MAX) / CHARS_PER_TOKEN;
        let fits = tokens <= window_tokens / SHARE && self.specs.len() <= MOST_LISTED;
        if connected.is_empty() || fits {
            return;
        }
        let mut connectors: Vec<(&str, Vec<&ToolSpec>)> = Vec::new();
        for spec in &connected {
            let Some(connector) = connector_of(&spec.name) else {
                continue;
            };
            match connectors.iter_mut().find(|(known, _)| *known == connector) {
                Some((_, tools)) => tools.push(spec),
                None => connectors.push((connector, vec![spec])),
            }
        }
        let mut framed = vec![search_spec()];
        framed.extend(
            connectors.iter().map(|(connector, tools)| {
                wrapper_spec(connector, tools, self.about.get(*connector))
            }),
        );
        // In the connected tools' place, the first of them standing for them all.
        let mut framed = Some(framed);
        let mut specs = Vec::new();
        for spec in std::mem::take(&mut self.specs) {
            if self.is_connected(&spec.name) && connector_of(&spec.name).is_some() {
                specs.extend(framed.take().into_iter().flatten());
            } else {
                specs.push(spec);
            }
        }
        self.specs = specs;
    }

    /// These tools with each connector's own told again from the connected tools on offer
    /// here: a subagent's, which may have fewer of them than its parent's.
    pub(super) fn told_again(mut self) -> Self {
        if !self.searching() {
            return self;
        }
        let specs = std::mem::take(&mut self.specs);
        self.specs = (specs.into_iter())
            .map(|spec| match connector_called(&spec.name) {
                Some(connector) if !self.by_name.contains_key(&spec.name) => {
                    let tools: Vec<&ToolSpec> = (self.catalog.iter())
                        .filter(|name| connector_of(name) == Some(connector))
                        .filter_map(|name| Some(&self.get(name)?.1.spec))
                        .collect();
                    wrapper_spec(connector, &tools, self.about.get(connector))
                }
                _ => spec,
            })
            .collect();
        self
    }

    /// Whether connectors are listed, each as one tool, with `tool_search`.
    fn searching(&self) -> bool {
        !self.by_name.contains_key(TOOL_SEARCH) && self.specs.iter().any(|s| s.name == TOOL_SEARCH)
    }

    /// The connector a listed connector's tool called `name` calls.
    fn wrapper_of<'a>(&self, name: &'a str) -> Option<&'a str> {
        let connector = connector_called(name)?;
        let listed = self.specs.iter().any(|spec| spec.name == name);
        (listed && !self.by_name.contains_key(name)).then_some(connector)
    }

    /// Whether a list of these tools keeps the one called `name` when its connected tools are
    /// `catalog`: `tool_search` while any is left, and a connector's tool while it has one.
    pub(super) fn lists_for(&self, name: &str, catalog: &[String]) -> bool {
        if !self.searching() {
            return false;
        }
        if name == TOOL_SEARCH {
            return !catalog.is_empty();
        }
        self.wrapper_of(name).is_some_and(|connector| {
            (catalog.iter()).any(|tool| connector_of(tool) == Some(connector))
        })
    }

    /// What `call` comes to: run as it is, run as the connected tool it stands for, a search,
    /// or refused. Only where connectors are listed is it ever more than run as it is.
    pub(crate) fn route(&self, call: &ToolCall) -> Route {
        if !self.searching() {
            return Route::Run(call.clone());
        }
        let name = call.tool.trim();
        let map = map_of(&call.arguments);
        if name.eq_ignore_ascii_case(TOOL_SEARCH) {
            return search_of(map, None);
        }
        if let Some(connector) = self.wrapper_of(name) {
            return self.through(call, connector);
        }
        if !self.is_connected(name) {
            // `mcp__<connector>__tool_search`, as a model may name the search within one.
            let searched = connector_of(name).filter(|connector| {
                let searches = [TOOL_SEARCH, "search"];
                searches
                    .iter()
                    .any(|s| short_of(name).eq_ignore_ascii_case(s))
                    && self.wrapper_of(&wrapper(connector)).is_some()
            });
            return match (searched, self.get(name)) {
                (Some(connector), None) => search_of(map, Some(connector)),
                _ => Route::Run(call.clone()),
            };
        }
        // Called by its own name: as its connector's tool is called, when its arguments are
        // shaped so (naming it, or only `arguments`) and it has no such parameters.
        let (_, info) = self.get(name).unzip();
        let has = |key: &str| {
            info.is_some_and(|info| {
                let parameter = format!("/properties/{key}");
                info.spec.parameters.pointer(&parameter).is_some()
            })
        };
        let shaped = |map: &Map<String, Value>| {
            let names_itself =
                text_of(map, "tool").is_some_and(|tool| tool == name || tool == short_of(name));
            let only_arguments = map.len() == 1 && map.contains_key("arguments");
            !has("tool") && !has("arguments") && (names_itself || only_arguments)
        };
        match (map, connector_of(name)) {
            (Some(map), Some(connector)) if shaped(map) => {
                let mut map = map.clone();
                map.insert("tool".to_owned(), Value::String(name.to_owned()));
                let call = ToolCall {
                    arguments: Arguments::Json(map),
                    ..call.clone()
                };
                self.through(&call, connector)
            }
            _ => self.checked(call.clone()),
        }
    }

    /// `call`, of `connector`'s tool, as the call of the tool it names.
    fn through(&self, call: &ToolCall, connector: &str) -> Route {
        let shaped = || {
            Route::Refused(ToolOutput::text(
                true,
                format!(
                    "Nothing ran. Call {} with the tool's name in `tool` and its arguments in \
                     `arguments`, as JSON.",
                    wrapper(connector)
                ),
            ))
        };
        let Some(map) = map_of(&call.arguments) else {
            return shaped();
        };
        let Some(tool) = text_of(map, "tool") else {
            return shaped();
        };
        let short = tool
            .strip_prefix(&format!("{}__", wrapper(connector)))
            .unwrap_or(tool);
        let named = if tool.starts_with("mcp__") && connector_of(tool).is_some() {
            tool.to_owned()
        } else {
            format!("{}__{short}", wrapper(connector))
        };
        let searches = [TOOL_SEARCH, "search"]
            .iter()
            .any(|s| short.eq_ignore_ascii_case(s));
        if searches && self.get(&named).is_none() {
            return search_of(Some(map), Some(connector));
        }
        let arguments = match map.get("arguments") {
            Some(Value::Object(inner)) => inner.clone(),
            Some(Value::String(text)) => match serde_json::from_str(text) {
                Ok(inner) => inner,
                Err(_) => return shaped(),
            },
            Some(Value::Null) | None => (map.iter())
                .filter(|(key, _)| *key != "tool" && *key != "arguments")
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            Some(_) => return shaped(),
        };
        if !self.is_connected(&named) {
            return Route::Refused(ToolOutput::text(true, self.missing_from(connector, short)));
        }
        self.checked(ToolCall {
            call_id: call.call_id.clone(),
            tool: named,
            arguments: Arguments::Json(arguments),
        })
    }

    /// What a call of `tool` through `connector`'s tool is told when there is no such tool.
    fn missing_from(&self, connector: &str, tool: &str) -> String {
        let has_any = (self.catalog.iter()).any(|name| connector_of(name) == Some(connector));
        if !has_any {
            return not_available(&wrapper(connector));
        }
        format!(
            "Nothing ran: {connector} has no tool called {tool:?}. Find the one you need with \
             {TOOL_SEARCH} (connector \"{connector}\"), then call it by the name it shows."
        )
    }

    /// `call`, of a connected tool, to run when its arguments fit the tool's parameters; else
    /// refused with them, so the model can call it again rightly.
    fn checked(&self, call: ToolCall) -> Route {
        let Some((_, info)) = self.get(&call.tool) else {
            return Route::Run(call);
        };
        let problem = match &call.arguments {
            Arguments::Json(map) => fits::problem(&info.spec.parameters, map),
            Arguments::Malformed(_) => Some("they are not valid JSON".to_owned()),
        };
        match problem {
            None => Route::Run(call),
            Some(problem) => Route::Refused(ToolOutput::text(
                true,
                format!(
                    "Nothing ran: the arguments for {} don't fit its parameters: {problem}. \
                     Its parameters: {}",
                    info.spec.name, info.spec.parameters
                ),
            )),
        }
    }

    /// Runs a search for `query`, of `connector`'s tools or all of them: their full definitions,
    /// the best first, at most eight.
    pub(crate) fn search(&self, query: Option<&str>, connector: Option<&str>) -> ToolOutput {
        let Some(query) = query else {
            return ToolOutput::text(true, "Say what to search for, in query.");
        };
        let connector = connector.map(|named| {
            let named = named.trim();
            named.strip_prefix("mcp__").unwrap_or(named)
        });
        let tools: Vec<&ToolSpec> = (self.catalog.iter())
            .filter(|name| connector.is_none_or(|c| connector_of(name) == Some(c)))
            .filter_map(|name| Some(&self.get(name)?.1.spec))
            .collect();
        if let Some(connector) = connector
            && tools.is_empty()
        {
            return ToolOutput::text(true, self.no_connector(connector));
        }
        let found = rank::matching(&tools, query);
        if found.is_empty() {
            let text = format!(
                "No connected tool matches {query:?}. Try other words, or another connector."
            );
            return ToolOutput::text(false, text);
        }
        ToolOutput::text(false, found_text(&found))
    }

    /// What a search of `connector`'s tools is told when it has none on offer.
    fn no_connector(&self, connector: &str) -> String {
        if self.wrapper_of(&wrapper(connector)).is_some() {
            return not_available(&wrapper(connector));
        }
        let mut known: Vec<&str> = Vec::new();
        for name in self.catalog.iter().filter_map(|name| connector_of(name)) {
            if !known.contains(&name) {
                known.push(name);
            }
        }
        format!(
            "There is no connector called {connector:?}. The connectors are {}.",
            known.join(", ")
        )
    }
}

#[cfg(test)]
mod tests;
