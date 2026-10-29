//! One request to a server that may take a while: answered within its limits, kept alive by
//! the word a long one sends of its progress, and cancelled with the turn. A call of a tool may
//! also be answered with what the server needs first (MCP 2026-07-28's input rounds), which
//! glave gives it before asking again.

use std::time::Duration;

use glave_ports::{Cancellation, unless_cancelled};
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, CallToolResult, CancelledNotificationParam,
    ClientRequest, DEFAULT_MRTR_MAX_ROUNDS, InputRequest, InputRequiredResult, InputResponses,
    ServerResult,
};
use rmcp::service::{Peer, PeerRequestOptions, RoleClient, ServiceError};

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
pub(crate) type Called = Result<CallToolResult, Unanswered>;

/// Calls a tool with `params` on `server`'s `peer`, within `waits` for each round, giving the
/// server what it asks for along the way from `client`, until it answers.
pub(crate) async fn call(
    server: &str,
    peer: &Peer<RoleClient>,
    client: &Client,
    mut params: CallToolRequestParams,
    cancel: &dyn Cancellation,
    waits: Waits,
) -> Called {
    for _ in 0..DEFAULT_MRTR_MAX_ROUNDS {
        let request = ClientRequest::CallToolRequest(CallToolRequest::new(params.clone()));
        match send(server, peer, request, cancel, waits).await? {
            ServerResult::CallToolResult(result) => return Ok(result),
            ServerResult::InputRequiredResult(asked) => {
                let (responses, state) = answer(server, client, asked)?;
                params.input_responses = responses;
                params.request_state = state;
            }
            _ => {
                return Err(Unanswered::Failed(format!(
                    "{server} answered something else."
                )));
            }
        }
    }
    Err(Unanswered::Failed(format!(
        "{server} kept asking for more, {DEFAULT_MRTR_MAX_ROUNDS} times, without answering."
    )))
}

/// Sends `request` to `server`'s `peer` and waits for its answer within `waits`, unless the
/// turn is cancelled first.
pub(crate) async fn send(
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
        Some(Err(ServiceError::Timeout { timeout })) if timeout == waits.quiet => Err(
            Unanswered::Failed(format!("{server} did not answer within {timeout:?}.")),
        ),
        Some(Err(ServiceError::Timeout { timeout })) => Err(Unanswered::Failed(format!(
            "{server} was still working after {timeout:?}, so it was stopped."
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

/// What `server` is given for what it `asked`, and the state it asked to have back.
fn answer(
    server: &str,
    client: &Client,
    asked: InputRequiredResult,
) -> Result<(Option<InputResponses>, Option<String>), Unanswered> {
    let mut responses = InputResponses::new();
    for (key, request) in asked.input_requests.unwrap_or_default() {
        let response = match request {
            InputRequest::ListRoots(_) => serde_json::to_value(client.roots()),
            _ => {
                return Err(Unanswered::Failed(format!(
                    "{server} asked for more input, which glave cannot give it yet."
                )));
            }
        };
        let response = response.map_err(|err| Unanswered::Failed(format!("{server}: {err}")))?;
        responses.insert(key, response);
    }
    Ok((
        (!responses.is_empty()).then_some(responses),
        asked.request_state,
    ))
}
