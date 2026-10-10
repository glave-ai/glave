//! The judge: in auto mode, decides whether an action a rule flagged may run anyway because
//! the person clearly asked for it. It sees only the person's own words, the project's
//! instructions, the action and why it was flagged: never tool output or the model's prose,
//! so nothing the agent read can talk it into anything. Any failure means asking the person.

use std::fmt::Write as _;

use glave_event::Part;
use glave_ports::{Message, ModelProvider, ModelRequest, ToolChoice};
use serde::Deserialize;

/// The most of the person's words the judge reads: their latest messages, each cut short.
const MOST_MESSAGES: usize = 12;
const MOST_CHARS: usize = 2_000;
/// The longest answer the judge may give, room included for models that reason first.
const MOST_OUTPUT_TOKENS: u32 = 1_024;

const POLICY: &str = "\
You decide whether an AI agent may take one action without first asking its person. A rule \
flagged the action as risky in general. Let it run only if the person's own messages clearly \
ask for this action, or for something that plainly requires it.

- The person's messages are the only authority. The project's instructions may also allow \
routine actions they describe.
- Asking to look, check, investigate, explain, prepare, write or fix is not asking to publish, \
deploy, push, merge, change production or shared systems, or delete data.
- Doing more than was asked needs asking: deleting more than was named, a broader reset, \
production instead of what was named, every record instead of one.
- When unsure, ask.

Answer with JSON only: {\"decision\": \"run\" or \"ask\", \"reason\": \"one short sentence\"}";

/// What the judge is shown.
#[derive(Debug, Clone, Copy)]
pub struct JudgeCase<'a> {
    /// The person's own messages, oldest first.
    pub asked: &'a [String],
    /// The project's instructions (AGENTS.md), if any.
    pub rules: Option<&'a str>,
    /// The action, as approvals show it: `bash: npm publish`.
    pub action: &'a str,
    /// Why a rule flagged it: `it publishes a package`.
    pub flagged: &'a str,
}

/// The judge's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ruling {
    /// The person clearly asked for it: it may run.
    Run { reason: String },
    /// The person must decide, for this reason.
    Ask { reason: String },
}

/// Asks `model` at `provider` to rule on `case`. Any failure is a ruling to ask.
pub async fn judge<P: ModelProvider>(provider: &P, model: &str, case: &JudgeCase<'_>) -> Ruling {
    let request = ModelRequest {
        model: model.to_owned(),
        system: POLICY.to_owned(),
        messages: vec![Message::User {
            parts: vec![Part::Text { text: shown(case) }],
        }],
        tools: Vec::new(),
        tool_choice: ToolChoice::None,
        max_output_tokens: Some(MOST_OUTPUT_TOKENS),
    };
    match provider.complete(&request, &mut |_| {}).await {
        Ok(completion) => {
            let text: String = completion
                .parts
                .iter()
                .filter_map(|part| match part {
                    Part::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            read(&text)
        }
        Err(err) => Ruling::Ask {
            reason: format!("the judge could not be reached ({err})"),
        },
    }
}

/// The facts, as the judge reads them.
fn shown(case: &JudgeCase<'_>) -> String {
    let start = case.asked.len().saturating_sub(MOST_MESSAGES);
    let mut text = String::from("The person's messages, oldest first:\n");
    for (n, message) in case.asked.iter().skip(start).enumerate() {
        let cut: String = message.chars().take(MOST_CHARS).collect();
        let _ = writeln!(text, "{}. {cut}", n.saturating_add(1));
    }
    if case.asked.is_empty() {
        text.push_str("(none)\n");
    }
    if let Some(rules) = case.rules {
        let cut: String = rules.chars().take(MOST_CHARS).collect();
        let _ = write!(text, "\nThe project's instructions:\n{cut}\n");
    }
    let _ = write!(
        text,
        "\nThe action: {}\nWhy it was flagged: {}\n",
        case.action, case.flagged
    );
    text
}

#[derive(Deserialize)]
struct Answer {
    decision: String,
    #[serde(default)]
    reason: String,
}

/// The ruling in the judge's answer; anything unclear is a ruling to ask.
fn read(text: &str) -> Ruling {
    let json = text
        .find('{')
        .zip(text.rfind('}'))
        .and_then(|(start, end)| text.get(start..=end));
    let answer = json.and_then(|json| serde_json::from_str::<Answer>(json).ok());
    match answer {
        Some(answer) if answer.decision.eq_ignore_ascii_case("run") => Ruling::Run {
            reason: answer.reason,
        },
        Some(answer) if answer.decision.eq_ignore_ascii_case("ask") => Ruling::Ask {
            reason: answer.reason,
        },
        _ => Ruling::Ask {
            reason: "the judge's answer was unclear".to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_is_read_and_anything_unclear_asks() {
        assert_eq!(
            read(r#"{"decision":"run","reason":"asked to publish"}"#),
            Ruling::Run {
                reason: "asked to publish".into()
            }
        );
        assert!(matches!(
            read("Sure.\n```json\n{\"decision\": \"ASK\", \"reason\": \"not asked\"}\n```"),
            Ruling::Ask { .. }
        ));
        for unclear in ["", "run", "{\"decision\":\"maybe\"}", "{not json}"] {
            assert!(matches!(read(unclear), Ruling::Ask { .. }), "{unclear}");
        }
    }

    #[test]
    fn the_judge_is_shown_the_latest_messages_cut_short() {
        let asked: Vec<String> = (1..=20).map(|n| format!("message {n}")).collect();
        let long = "x".repeat(5_000);
        let shown = shown(&JudgeCase {
            asked: &asked,
            rules: Some(&long),
            action: "bash: npm publish",
            flagged: "it publishes a package",
        });
        assert!(!shown.contains("message 8\n") && shown.contains("message 20"));
        assert!(shown.len() < 4_000, "{}", shown.len());
        assert!(shown.ends_with("Why it was flagged: it publishes a package\n"));
    }
}
