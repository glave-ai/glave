//! A model that answers from a script, for tests of what drives a model: the harness, the
//! session, memory's tidier, and the benchmarks.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use glave_event::Part;
use tokio::sync::Notify;

use crate::model::{Completion, Delta, ModelProvider, ModelRequest, ProviderError};

/// Every request a scripted model received, in order.
pub type Seen = Arc<Mutex<Vec<ModelRequest>>>;

/// One scripted answer: a completion, or the error the provider gives.
pub type Answer = Result<Completion, ProviderError>;

/// A model answering with scripted completions (or errors) in turn, streaming each answer's
/// text and reasoning as deltas first, and keeping every request it is sent. One that has run out
/// fails with a protocol error, which is not transient, so a test never waits through retries;
/// a cycling one starts its script again instead.
#[derive(Debug)]
pub struct ScriptedModel {
    answers: Mutex<VecDeque<Answer>>,
    /// What a cycling model answers again once it runs out; nothing for one that fails.
    again: Vec<Answer>,
    gate: Option<Arc<Notify>>,
    seen: Seen,
    recording: bool,
}

impl ScriptedModel {
    /// A model answering `answers` in order, then failing.
    pub fn new(answers: Vec<Completion>) -> Self {
        Self::answering(answers.into_iter().map(Ok).collect())
    }

    /// A model answering each of `answers` in order, a completion or an error, then failing.
    pub fn answering(answers: Vec<Answer>) -> Self {
        Self {
            answers: Mutex::new(answers.into()),
            again: Vec::new(),
            gate: None,
            seen: Seen::default(),
            recording: true,
        }
    }

    /// A model answering `answers` in order, and from the first again once they run out.
    pub fn cycling(answers: Vec<Completion>) -> Self {
        let again: Vec<Answer> = answers.into_iter().map(Ok).collect();
        Self {
            again: again.clone(),
            ..Self::answering(again)
        }
    }

    /// The same model, waiting for `gate` to be notified before it finishes each answer.
    #[must_use]
    pub fn gated(self, gate: Arc<Notify>) -> Self {
        Self {
            gate: Some(gate),
            ..self
        }
    }

    /// The same model, keeping no requests: for a benchmark, whose conversations grow long.
    #[must_use]
    pub fn unrecorded(self) -> Self {
        Self {
            recording: false,
            ..self
        }
    }

    /// Every request the model receives, as it receives them.
    pub fn seen(&self) -> Seen {
        Arc::clone(&self.seen)
    }

    fn next(&self) -> Option<Answer> {
        let mut answers = self.answers.lock().unwrap_or_else(PoisonError::into_inner);
        if answers.is_empty() {
            answers.extend(self.again.iter().cloned());
        }
        answers.pop_front()
    }
}

impl ModelProvider for ScriptedModel {
    fn complete(
        &self,
        request: &ModelRequest,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> impl Future<Output = Result<Completion, ProviderError>> + Send {
        if self.recording {
            self.seen
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(request.clone());
        }
        let next = self.next();
        let parts = next
            .iter()
            .flat_map(|answer| answer.iter())
            .flat_map(|answer| &answer.parts);
        for part in parts {
            match part {
                Part::Text { text } => on_delta(Delta::Text(text.clone())),
                Part::Reasoning {
                    text: Some(text), ..
                } => on_delta(Delta::Reasoning(text.clone())),
                Part::Reasoning { text: None, .. } | Part::File { .. } => {}
            }
        }
        let gate = self.gate.clone();
        async move {
            if let Some(gate) = gate {
                gate.notified().await;
            }
            next.unwrap_or_else(|| Err(ProviderError::Protocol("script exhausted".into())))
        }
    }
}

#[cfg(test)]
mod tests {
    use glave_event::{ModelRef, StopReason, TokenCounts, Usage};

    use super::*;
    use crate::block_on;

    fn said(text: &str) -> Completion {
        Completion {
            model: ModelRef {
                provider: "script".into(),
                name: "test-model".into(),
            },
            response_id: None,
            parts: vec![Part::Text { text: text.into() }],
            tool_calls: Vec::new(),
            stop: StopReason::EndTurn,
            provider_stop: None,
            usage: Usage::new(TokenCounts::default(), false, None).unwrap(),
            cost: None,
            served: None,
        }
    }

    fn request() -> ModelRequest {
        serde_json::from_value(serde_json::json!({
            "model": "test-model", "system": "", "messages": [], "tools": []
        }))
        .unwrap()
    }

    fn ask(model: &ScriptedModel) -> (Result<Completion, ProviderError>, Vec<Delta>) {
        let mut deltas = Vec::new();
        let result = block_on!(model.complete(&request(), &mut |delta| deltas.push(delta)));
        (result, deltas)
    }

    #[test]
    fn answers_stream_then_run_out_and_every_request_is_kept() {
        let model = ScriptedModel::new(vec![said("One."), said("Two.")]);
        let seen = model.seen();
        let (first, deltas) = ask(&model);
        assert_eq!(first.unwrap(), said("One."));
        assert_eq!(deltas, [Delta::Text("One.".into())]);
        assert_eq!(ask(&model).0.unwrap(), said("Two."));
        let (out, deltas) = ask(&model);
        assert!(matches!(out, Err(ProviderError::Protocol(_))), "{out:?}");
        assert_eq!(deltas, []);
        assert_eq!(seen.lock().unwrap().len(), 3);
    }

    #[test]
    fn a_cycling_script_starts_again() {
        let model = ScriptedModel::cycling(vec![said("One."), said("Two.")]);
        let answers: Vec<Completion> = (0..3).map(|_| ask(&model).0.unwrap()).collect();
        assert_eq!(answers, [said("One."), said("Two."), said("One.")]);
        let unrecorded = ScriptedModel::cycling(vec![said("One.")]).unrecorded();
        ask(&unrecorded).0.unwrap();
        assert!(unrecorded.seen().lock().unwrap().is_empty());
    }

    #[test]
    fn an_error_is_answered_in_its_turn_without_deltas() {
        let refused = ProviderError::Http {
            status: 400,
            body: "too long".into(),
        };
        let model = ScriptedModel::answering(vec![Err(refused.clone()), Ok(said("Short."))]);
        let (first, deltas) = ask(&model);
        assert_eq!(first, Err(refused));
        assert_eq!(deltas, []);
        assert_eq!(ask(&model).0.unwrap(), said("Short."));
    }

    #[test]
    fn a_gated_answer_waits_for_the_gate() {
        let gate = Arc::new(Notify::new());
        let model = ScriptedModel::new(vec![said("Done.")]).gated(Arc::clone(&gate));
        gate.notify_one();
        assert_eq!(ask(&model).0.unwrap(), said("Done."));
    }
}
