//! What a server asks the person inside a call (MCP's elicitation, in MCP 2026-07-28's input
//! rounds): glave has no way to reach them yet, so it declines for them, and the call goes on.

use std::collections::BTreeMap;

use glave_event::Part;
use glave_ports::ToolOutput;
use rmcp::model::{
    ElicitResult, ElicitationAction, InputRequest, InputRequests, InputRequiredResult,
    InputResponses,
};
use serde_json::Value;

use crate::call::Unanswered;
use crate::client::Client;

/// What the call has come to across its rounds.
#[derive(Debug, Default)]
pub(crate) struct Asking {
    /// How many times the server has been called.
    pub(crate) rounds: usize,
    /// The state it asked to have back, echoed as it was.
    pub(crate) state: Option<String>,
    /// What goes back with the next call.
    pub(crate) responses: InputResponses,
    /// Whether it asked the person something, which glave declined.
    declined: bool,
}

impl Asking {
    /// Takes in what `server` asked in its last round: all of it goes back with the next call.
    pub(crate) fn asked(
        &mut self,
        server: &str,
        client: &Client,
        asked: InputRound,
    ) -> Result<(), Unanswered> {
        let bad = |err: serde_json::Error| Unanswered::Failed(format!("{server}: {err}"));
        self.state = asked.state;
        for (key, request) in asked.requests {
            let response = match request {
                Asked::Readable(request) if matches!(*request, InputRequest::ListRoots(_)) => {
                    serde_json::to_value(client.roots()).map_err(bad)?
                }
                Asked::Readable(request) if !matches!(*request, InputRequest::Elicitation(_)) => {
                    return Err(Unanswered::Failed(format!(
                        "{server} asked for more input, which glave cannot give it yet."
                    )));
                }
                Asked::Readable(_) | Asked::Unreadable => {
                    self.declined = true;
                    serde_json::to_value(ElicitResult::new(ElicitationAction::Decline))
                        .map_err(bad)?
                }
            };
            self.responses.insert(key, response);
        }
        Ok(())
    }

    /// `output`, with what the model is told of what was asked before it.
    pub(crate) fn told(&self, server: &str, mut output: ToolOutput) -> ToolOutput {
        if let Some(Part::Text { text }) = output.parts.first_mut() {
            *text = self.told_with(server, std::mem::take(text));
        }
        output
    }

    /// `why`, an error in the server's words, with what the model is told of what was asked.
    pub(crate) fn told_with(&self, server: &str, why: String) -> String {
        if self.declined {
            format!(
                "[{server} asked the person something, which glave cannot put to them yet, so \
                 it was declined.]\n{why}"
            )
        } else {
            why
        }
    }
}

/// A server's input round, as far as it could be read.
pub(crate) struct InputRound {
    requests: BTreeMap<String, Asked>,
    state: Option<String>,
}

/// One request of a round.
enum Asked {
    Readable(Box<InputRequest>),
    /// A form glave cannot read, as one with a field whose format MCP does not know
    /// (`"format": "password"`).
    Unreadable,
}

impl From<InputRequiredResult> for InputRound {
    fn from(asked: InputRequiredResult) -> Self {
        let requests: InputRequests = asked.input_requests.unwrap_or_default();
        let requests = requests.into_iter();
        Self {
            requests: requests
                .map(|(key, request)| (key, Asked::Readable(Box::new(request))))
                .collect(),
            state: asked.request_state,
        }
    }
}

impl InputRound {
    /// The round in `result`, a result rmcp could not read as any it knows, when it is one: a
    /// form it cannot read makes the whole result so. Each request is read on its own. `None`
    /// when it is not a round, or holds what is not a form.
    pub(crate) fn unread(result: &Value) -> Option<Self> {
        if result.get("resultType")?.as_str()? != "input_required" {
            return None;
        }
        let state = result.get("requestState").and_then(Value::as_str);
        let listed = result.get("inputRequests").and_then(Value::as_object);
        let mut requests = BTreeMap::new();
        for (key, request) in listed.into_iter().flatten() {
            let asked = match serde_json::from_value(request.clone()) {
                Ok(readable) => Asked::Readable(Box::new(readable)),
                Err(_) if request.get("method")?.as_str()? == "elicitation/create" => {
                    Asked::Unreadable
                }
                Err(_) => return None,
            };
            requests.insert(key.clone(), asked);
        }
        Some(Self {
            requests,
            state: state.map(str::to_owned),
        })
    }
}

#[cfg(test)]
mod tests {
    use rmcp::model::ServerResult;
    use serde_json::json;

    use super::*;

    #[test]
    fn a_form_rmcp_cannot_read_is_read_on_its_own() {
        let round = json!({
            "resultType": "input_required",
            "requestState": "kept",
            "inputRequests": {
                "secret": {"method": "elicitation/create", "params": {
                    "mode": "form", "message": "Your password", "requestedSchema": {
                        "type": "object",
                        "properties": {"p": {"type": "string", "format": "password"}}
                    }
                }},
                "fine": {"method": "elicitation/create", "params": {
                    "mode": "form", "message": "Which?", "requestedSchema": {
                        "type": "object", "properties": {"go": {"type": "boolean"}}
                    }
                }}
            }
        });
        let result: ServerResult = serde_json::from_value(round).unwrap();
        let ServerResult::CustomResult(custom) = result else {
            panic!("rmcp read it as {result:?}");
        };
        let round = InputRound::unread(&custom.0).unwrap();
        assert!(matches!(
            round.requests.get("secret"),
            Some(Asked::Unreadable)
        ));
        assert!(matches!(
            round.requests.get("fine"),
            Some(Asked::Readable(_))
        ));
        assert_eq!(round.state.as_deref(), Some("kept"));
        assert!(InputRound::unread(&json!({"resultType": "complete"})).is_none());
        assert!(InputRound::unread(&json!({"anything": 1})).is_none());
    }

    #[test]
    fn the_model_is_told_it_was_declined_only_when_it_was() {
        let asking = Asking::default();
        let said = asking.told("github", ToolOutput::text(false, "done"));
        assert_eq!(said, ToolOutput::text(false, "done"));
        let asking = Asking {
            declined: true,
            ..Asking::default()
        };
        let said = asking.told("github", ToolOutput::text(false, "done"));
        let [Part::Text { text }] = said.parts.as_slice() else {
            panic!("{said:?}");
        };
        assert!(
            text.starts_with("[github asked the person something"),
            "{text}"
        );
        assert!(text.ends_with("\ndone"), "{text}");
    }
}
