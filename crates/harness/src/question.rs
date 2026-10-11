//! `question`: questions for the person, each with choices or none, as Claude Code's
//! AskUserQuestion, Codex's request_user_input and OpenCode's question put them. It is offered
//! only when someone can answer; the question waits in the call, so the log keeps it and any
//! client can show it.

use std::fmt::Write as _;

use glave_event::{Arguments, CallId, ThreadId};
use glave_ports::ToolSpec;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// The tool's name.
pub const QUESTION: &str = "question";

/// Most questions in one call.
const MOST: usize = 4;

/// One question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Question {
    pub question: String,
    /// Choices to pick from; the person may answer in their own words either way.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<Choice>,
    /// Whether more than one choice may be picked.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub multiple: bool,
}

/// One choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Questions the model put to the person, waiting in a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionRequest {
    pub thread: ThreadId,
    /// The call waiting, which is also the question's id.
    pub call_id: CallId,
    pub questions: Vec<Question>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Asked {
    questions: Vec<Question>,
}

pub(crate) fn spec() -> ToolSpec {
    ToolSpec {
        name: QUESTION.to_owned(),
        description: "Ask the person when you need a decision or a fact only they have, and \
                      cannot find it in the project. Give choices when there are clear ones; \
                      they can always answer in their own words. Ask up to 4 questions at once."
            .to_owned(),
        parameters: json!({"type": "object", "properties": {
            "questions": {"type": "array", "maxItems": MOST, "items": {"type": "object", "properties": {
                "question": {"type": "string"},
                "options": {"type": "array", "items": {"type": "object", "properties": {
                    "label": {"type": "string"},
                    "description": {"type": "string"}
                }, "required": ["label"]}},
                "multiple": {"type": "boolean", "description": "More than one choice may be picked"}
            }, "required": ["question"]}}
        }, "required": ["questions"]}),
    }
}

/// The questions in a call, or why they could not be read.
pub(crate) fn questions(arguments: &Arguments) -> Result<Vec<Question>, String> {
    let Arguments::Json(arguments) = arguments else {
        return Err("The arguments were not valid JSON.".to_owned());
    };
    let asked: Asked = serde_json::from_value(serde_json::Value::Object(arguments.clone()))
        .map_err(|err| format!("The questions were not understood: {err}."))?;
    match asked.questions.len() {
        0 => Err("Ask at least one question.".to_owned()),
        n if n > MOST => Err(format!("Ask at most {MOST} questions at once, not {n}.")),
        _ => Ok(asked.questions),
    }
}

/// What the model is told: each question with the person's answer.
pub(crate) fn answered(questions: &[Question], answers: &[String]) -> String {
    let mut text = String::from("The person answered:");
    for (index, question) in questions.iter().enumerate() {
        let answer = answers
            .get(index)
            .map(String::as_str)
            .filter(|answer| !answer.trim().is_empty())
            .unwrap_or("(no answer)");
        let _ = write!(text, "\n- {} → {answer}", question.question);
    }
    text
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
    fn questions_are_read_with_their_choices() {
        let asked = arguments(json!({"questions": [
            {"question": "Which database?", "options": [
                {"label": "Postgres", "description": "what production uses"},
                {"label": "SQLite"}
            ]},
            {"question": "Anything else?"}
        ]}));
        let questions = questions(&asked).unwrap();
        assert_eq!(questions.len(), 2);
        assert_eq!(questions[0].options[1].label, "SQLite");
        assert!(!questions[0].multiple);
        assert_eq!(
            answered(&questions, &["Postgres".to_owned(), String::new()]),
            "The person answered:\n- Which database? → Postgres\n- Anything else? → (no answer)"
        );
    }

    #[test]
    fn too_few_too_many_or_odd_questions_say_why() {
        let none = arguments(json!({"questions": []}));
        assert_eq!(questions(&none).unwrap_err(), "Ask at least one question.");
        let five = arguments(json!({"questions": [
            {"question": "1"}, {"question": "2"}, {"question": "3"}, {"question": "4"}, {"question": "5"}
        ]}));
        assert_eq!(
            questions(&five).unwrap_err(),
            "Ask at most 4 questions at once, not 5."
        );
        let odd = arguments(json!({"questions": [{"question": "x", "color": "red"}]}));
        assert!(
            questions(&odd)
                .unwrap_err()
                .contains("unknown field `color`")
        );
    }
}
