//! One request to a server that may take a while: answered within its limits, kept alive by
//! the word a long one sends of its progress, and cancelled with the turn. A call of a tool may
//! also be answered with what the server needs first (MCP 2026-07-28's input rounds), which
//! glave gives it before asking again, the person's part once they have given it (`asks.rs`).

use std::time::Duration;

use glave_ports::{Cancellation, ServiceGiven, spoken, unless_cancelled};
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, CallToolResult, CancelledNotificationParam,
    ClientRequest, CustomResult, DEFAULT_MRTR_MAX_ROUNDS, ServerResult,
};
use rmcp::service::{Peer, PeerRequestOptions, RoleClient, ServiceError};

use crate::asks::{Asking, Form, InputRound};
use crate::client::Client;
use crate::reason::failed;

/// How long one request may take.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Waits {
    /// Without a word from the server: an answer or its progress.
    pub(crate) quiet: Duration,
    /// In all, however often it says it is still working.
    pub(crate) total: Duration,
}

impl Waits {
    /// rmcp's options for one request: given up on after `quiet` without a word, or `total`
    /// in all, and the server told it was.
    fn options(self) -> PeerRequestOptions {
        PeerRequestOptions::with_timeout(self.quiet)
            .reset_timeout_on_progress()
            .with_max_total_timeout(self.total)
    }
}

/// Why a request that was not answered was not.
pub(crate) enum Unanswered {
    /// The person cancelled the turn; the server was told.
    Cancelled,
    /// The server answered with an error, its message in its own words.
    Refused(String),
    /// It gave no answer, in glave's words.
    Failed(String),
}

/// What a call of a tool came to.
pub(crate) enum Ran {
    /// The server's result, and what to tell the model of what was asked along the way.
    Done(CallToolResult, Asking),
    /// The server asks the person this, and the call waits: its form is the first in `asking`.
    Asks(Form, Asking),
}

/// Who answers what the server asks along the way.
pub(crate) enum Answerer {
    /// No one: glave declines.
    NoOne,
    /// The person, who has given this so far.
    Person(Option<ServiceGiven>),
}

/// Calls a tool with `params` on `server`'s `peer`, within `waits` for each round, giving the
/// server what it asks for along the way from `client` and `answerer`, until it answers or asks
/// the person.
#[expect(
    clippy::too_many_arguments,
    reason = "one call: where it goes, what it asks, who answers along the way, and its limits"
)]
pub(crate) async fn call(
    server: &str,
    peer: &Peer<RoleClient>,
    client: &Client,
    mut params: CallToolRequestParams,
    answerer: Answerer,
    cancel: &dyn Cancellation,
    waits: Waits,
) -> Result<Ran, Unanswered> {
    let bad = |err: serde_json::Error| Unanswered::Failed(format!("{server}: {err}"));
    let mut asking = Asking::default();
    let person = matches!(answerer, Answerer::Person(_));
    if let Answerer::Person(Some(given)) = answerer {
        asking = serde_json::from_value(given.state).map_err(bad)?;
        // What the person answered goes to the server they answered, and no other.
        if asking.server != server {
            return Err(Unanswered::Failed(format!(
                "{server} is not {}, which asked, so what the person answered was not sent.",
                asking.server
            )));
        }
        asking.given(given.answer).map_err(bad)?;
    }
    loop {
        if !person {
            asking.no_one_there().map_err(bad)?;
        }
        if let Some(form) = asking.forms.front() {
            return Ok(Ran::Asks(form.clone(), asking));
        }
        if asking.rounds >= DEFAULT_MRTR_MAX_ROUNDS {
            return Err(Unanswered::Failed(format!(
                "{server} kept asking for more, {DEFAULT_MRTR_MAX_ROUNDS} times, without answering."
            )));
        }
        asking.rounds = asking.rounds.saturating_add(1);
        let responses = std::mem::take(&mut asking.responses);
        params.input_responses = (!responses.is_empty()).then_some(responses);
        params.request_state = asking.state.take();
        let request = ClientRequest::CallToolRequest(CallToolRequest::new(params.clone()));
        let came = match send(server, peer, request, cancel, waits).await {
            Err(Unanswered::Refused(why)) => {
                return Err(Unanswered::Refused(asking.told_with(server, why)));
            }
            other => other?,
        };
        let round = match came {
            ServerResult::CallToolResult(result) => return Ok(Ran::Done(result, asking)),
            ServerResult::InputRequiredResult(asked) => Some(InputRound::from(asked)),
            ServerResult::CustomResult(CustomResult(result)) => InputRound::unread(&result),
            _ => None,
        };
        let round = round
            .ok_or_else(|| Unanswered::Failed(format!("{server} answered something else.")))?;
        asking.asked(server, client, round)?;
    }
}

/// Sends `request` to `server`'s `peer` and waits for its answer within `waits`, unless the
/// turn is cancelled first.
async fn send(
    server: &str,
    peer: &Peer<RoleClient>,
    request: ClientRequest,
    cancel: &dyn Cancellation,
    waits: Waits,
) -> Result<ServerResult, Unanswered> {
    let handle = peer
        .send_cancellable_request(request, waits.options())
        .await
        .map_err(|err| Unanswered::Failed(failed(server, &err)))?;
    let id = handle.id.clone();
    // Bounded by `waits`: rmcp gives up, and tells the server, when either runs out.
    match unless_cancelled(cancel, handle.await_response()).await {
        Some(Ok(result)) => Ok(result),
        Some(Err(ServiceError::Timeout { timeout })) if timeout == waits.quiet => {
            Err(Unanswered::Failed(format!(
                "{server} did not answer within {}.",
                spoken(timeout)
            )))
        }
        Some(Err(ServiceError::Timeout { timeout })) => Err(Unanswered::Failed(format!(
            "{server} was still working after {}, so it was stopped.",
            spoken(timeout)
        ))),
        Some(Err(ServiceError::TransportClosed)) => {
            Err(Unanswered::Failed(format!("{server} stopped answering.")))
        }
        Some(Err(err @ ServiceError::McpError(_))) => {
            Err(Unanswered::Refused(failed(server, &err)))
        }
        Some(Err(err)) => Err(Unanswered::Failed(failed(server, &err))),
        None => {
            let reason = Some("cancelled by the person".to_owned());
            let _ = peer
                .notify_cancelled(CancelledNotificationParam::new(Some(id), reason))
                .await;
            Err(Unanswered::Cancelled)
        }
    }
}
