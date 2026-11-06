//! Which connected tools a search is about: those it names, else the best by BM25 over each
//! tool's name, description and parameters, as Codex and Anthropic's own tool search rank.

use std::collections::HashMap;

use glave_ports::ToolSpec;
use serde_json::Value;

/// The most tools one search shows, as Codex's: enough that one search usually ends in one
/// call, few enough that their definitions stay small.
const MOST_FOUND: usize = 8;

/// BM25's usual constants: how soon more of a word stops counting, and how much a long
/// description is marked down for its length.
const K1: f64 = 1.2;
const B: f64 = 0.75;

/// Which of `tools` `query` finds: the ones it names, by their whole name or by their name
/// within their connector, when it is nothing but names, in `tools`' order; else the best by BM25, at most
/// [`MOST_FOUND`], the same way every time.
pub(super) fn matching<'a>(tools: &[&'a ToolSpec], query: &str) -> Vec<&'a ToolSpec> {
    let asked: Vec<&str> = query
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|word| !word.is_empty())
        .collect();
    let is_named = |tool: &ToolSpec, word: &str| {
        let short = super::short_of(&tool.name);
        tool.name.eq_ignore_ascii_case(word) || short.eq_ignore_ascii_case(word)
    };
    let every_word_names = asked
        .iter()
        .all(|word| tools.iter().any(|tool| is_named(tool, word)));
    if !asked.is_empty() && every_word_names {
        // Each tool of that name, in every connector that has one.
        let named = tools
            .iter()
            .filter(|tool| asked.iter().any(|word| is_named(tool, word)));
        return named.copied().take(MOST_FOUND).collect();
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

/// The indexes of `tools` that `query` is about, best first, by BM25 over each tool's name
/// (its connector's with it), which counts twice, its description and its parameters; a tie
/// stays in `tools`' order. Sentences are full of words most tools have, so where a query has
/// rarer words, a tool must have one of them, and a word no tool has counts as rarer. What
/// scores under half the best is left out, since it would only lengthen the result.
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
    let average = (lengths.iter().sum::<f64>() / count.max(1.0)).max(1.0);
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

/// `text`'s words, lowercase and without a plural's `s`, parted where it is neither a letter
/// nor a digit and inside a name in camelCase: `listOpenIssues` is `list`, `open`, `issue`.
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(name: &str, description: &str) -> ToolSpec {
        ToolSpec {
            name: name.to_owned(),
            description: description.to_owned(),
            parameters: json!({"type": "object", "properties": {"text": {"type": "string"}}}),
        }
    }

    fn found(tools: &[ToolSpec], query: &str) -> Vec<String> {
        let tools: Vec<&ToolSpec> = tools.iter().collect();
        matching(&tools, query)
            .iter()
            .map(|t| t.name.clone())
            .collect()
    }

    fn catalog() -> Vec<ToolSpec> {
        vec![
            tool("mcp__github__list_issues", "List a repository's issues."),
            tool("mcp__github__create_pull_request", "Open a pull request."),
            tool("mcp__slack__post_message", "Post a message to a channel."),
            tool(
                "mcp__pager__whoIsOnCall",
                "Find who is on call for a service.",
            ),
            tool("mcp__wiki__search", "Search pages. Mentions issues rarely."),
        ]
    }

    #[test]
    fn the_best_match_comes_first_and_only_what_matches_comes_at_all() {
        let tools = catalog();
        assert_eq!(
            found(&tools, "post to a slack channel")[0],
            "mcp__slack__post_message"
        );
        assert_eq!(found(&tools, "kubernetes"), Vec::<String>::new());
        assert_eq!(found(&tools, "issues")[0], "mcp__github__list_issues");
    }

    #[test]
    fn names_find_their_tools_whole_or_within_their_connector() {
        let tools = catalog();
        assert_eq!(
            found(&tools, "whoIsOnCall mcp__slack__post_message"),
            ["mcp__slack__post_message", "mcp__pager__whoIsOnCall"]
        );
    }

    #[test]
    fn a_sentence_finds_what_its_rare_words_find_and_nothing_for_its_common_ones() {
        let mut tools: Vec<ToolSpec> = (0..8)
            .map(|n| {
                tool(
                    &format!("mcp__a__report_{n}"),
                    "It is for a team, on a page.",
                )
            })
            .collect();
        tools.push(tool(
            "mcp__pager__whoIsOnCall",
            "Find who is on call for a service.",
        ));
        assert_eq!(
            found(&tools, "who is on call for payments"),
            ["mcp__pager__whoIsOnCall"]
        );
        assert_eq!(found(&tools, "kubernetes for a team"), Vec::<String>::new());
    }

    #[test]
    fn a_schema_s_own_words_find_nothing_but_its_parameters_names_do_at_most_eight() {
        let tools: Vec<ToolSpec> = (0..12)
            .map(|n| tool(&format!("mcp__a__t{n}"), "A tool."))
            .collect();
        assert_eq!(found(&tools, "object"), Vec::<String>::new());
        assert_eq!(found(&tools, "text").len(), MOST_FOUND);
        assert_eq!(
            found(&tools, "text"),
            found(&tools, "text"),
            "the same every time"
        );
    }
}
