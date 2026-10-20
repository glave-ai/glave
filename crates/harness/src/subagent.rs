//! `agent`: work handed to a subagent, a child thread, as Claude Code's Agent, Codex's
//! spawn_agent and OpenCode's task tool do it. The child starts fresh with only the prompt,
//! works with the parent's permissions or fewer, and its last answer comes back as the call's
//! result. Several in one response run side by side.

use glave_event::{Arguments, Outcome, Part};
use glave_ports::{Message, ToolSpec};
use serde::Deserialize;
use serde_json::json;

/// The tool's name.
pub(crate) const AGENT: &str = "agent";

/// Subagents running at once, as Codex caps them.
pub(crate) const AT_ONCE: usize = 6;

/// What a subagent may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubagentType {
    /// Every tool the parent has, except starting subagents and asking the person.
    #[default]
    General,
    /// Only tools that read.
    Explore,
}

/// One subagent, as the model asked for it.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Asked {
    /// A few words, for the person.
    pub(crate) description: String,
    pub(crate) prompt: String,
    #[serde(default)]
    pub(crate) subagent_type: SubagentType,
}

pub(crate) fn spec() -> ToolSpec {
    ToolSpec {
        name: AGENT.to_owned(),
        description: "Hand a self-contained piece of work to a helper that starts with no \
                      context but your prompt, and get its report back. Use it to search widely \
                      or do independent work without filling your own context. Several agent \
                      calls in one response run at the same time. The helper cannot see this \
                      conversation, so say everything it needs, and what to report."
            .to_owned(),
        parameters: json!({"type": "object", "properties": {
            "description": {"type": "string", "description": "The work in 3 to 6 words"},
            "prompt": {"type": "string", "description": "Everything the helper needs to know and do"},
            "subagent_type": {"type": "string", "enum": ["general", "explore"],
                      "description": "explore only reads and searches; general (the default) may also change files and run commands"}
        }, "required": ["description", "prompt"]}),
    }
}

/// The subagent a call asks for, or why it could not be read.
pub(crate) fn asked(arguments: &Arguments) -> Result<Asked, String> {
    let Arguments::Json(arguments) = arguments else {
        return Err("The arguments were not valid JSON.".to_owned());
    };
    let asked: Asked = serde_json::from_value(serde_json::Value::Object(arguments.clone()))
        .map_err(|err| format!("The request was not understood: {err}."))?;
    if asked.prompt.trim().is_empty() {
        return Err("Give the helper a prompt.".to_owned());
    }
    Ok(asked)
}

/// What the child is told: who it works for and what to hand back, then the prompt.
pub(crate) fn brief(asked: &Asked) -> String {
    format!(
        "[You are a helper doing one piece of work for another agent, who works with the \
         person. Only that agent sees your answers. Do the work, then end with a short report: \
         what you found or changed, with file paths, and anything left undone.]\n\n{}",
        asked.prompt
    )
}

/// The child's report: its last answer, with how it ended when it did not finish.
pub(crate) fn report(messages: &[Message], outcome: Outcome) -> String {
    let last = messages.iter().rev().find_map(|message| match message {
        Message::Assistant { parts, .. } => {
            let text: Vec<&str> = parts
                .iter()
                .filter_map(|part| match part {
                    Part::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            let text = text.join("\n");
            (!text.trim().is_empty()).then_some(text)
        }
        _ => None,
    });
    let text = last.unwrap_or_else(|| "(the helper gave no report)".to_owned());
    match outcome {
        Outcome::Completed => text,
        other => format!("[The helper stopped early: {}.]\n\n{text}", ended(other)),
    }
}

fn ended(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Cancelled => "cancelled",
        Outcome::LimitReached => "it reached its step limit",
        Outcome::Failed => "it failed",
        Outcome::Interrupted => "it was interrupted",
        Outcome::Completed => "it finished",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(value: serde_json::Value) -> Arguments {
        match value {
            serde_json::Value::Object(map) => Arguments::Json(map),
            other => panic!("not an object: {other}"),
        }
    }

    #[test]
    fn a_request_is_read_with_its_type() {
        let explore = asked(&arguments(json!({
            "description": "find the tests",
            "prompt": "List the test files.",
            "subagent_type": "explore"
        })))
        .unwrap();
        assert_eq!(explore.subagent_type, SubagentType::Explore);
        assert!(brief(&explore).ends_with("\n\nList the test files."));
        let general = asked(&arguments(json!({"description": "x", "prompt": "Do it."}))).unwrap();
        assert_eq!(general.subagent_type, SubagentType::General);
        let empty = asked(&arguments(json!({"description": "x", "prompt": " "})));
        assert_eq!(empty.unwrap_err(), "Give the helper a prompt.");
        let odd = asked(&arguments(
            json!({"description": "x", "prompt": "y", "subagent_type": "boss"}),
        ));
        assert!(odd.unwrap_err().contains("unknown variant `boss`"));
    }

    #[test]
    fn the_report_is_the_last_answer_and_says_how_it_ended() {
        let said = |text: &str| Message::Assistant {
            parts: vec![Part::Text { text: text.into() }],
            tool_calls: vec![],
        };
        let messages = vec![said("Looking."), said("Found 3 tests in tests/.")];
        assert_eq!(
            report(&messages, Outcome::Completed),
            "Found 3 tests in tests/."
        );
        assert_eq!(
            report(&messages, Outcome::LimitReached),
            "[The helper stopped early: it reached its step limit.]\n\nFound 3 tests in tests/."
        );
        assert_eq!(
            report(&[], Outcome::Completed),
            "(the helper gave no report)"
        );
    }
}
