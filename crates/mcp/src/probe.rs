//! Which protocol a server speaks, settled before rmcp's lifecycle starts. MCP 2026-07-28 has a
//! client that also speaks the older protocol ask `server/discover` first, and start the older
//! `initialize` handshake on an answer that is not the newer protocol's, or on none in a
//! reasonable time (stdio's "Backward Compatibility"). rmcp's automatic lifecycle takes 10 s
//! with no answer as "older" and never looks back, so a server on the newer protocol that was
//! slow to answer, a program still starting on a busy machine, was spoken to in the older one,
//! where glave answers no form (#1240). Here the probe waits 10 s alone, then the handshake is
//! offered beside it, and the first answer that says which protocol the server speaks decides:
//! a discovery to the probe for the newer one, a handshake for the older; a refused handshake
//! decides nothing. rmcp then starts on that protocol, its first request answered with the
//! answer already in, so nothing is asked twice, and the other request's answer is dropped.

use std::collections::VecDeque;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{
    ClientConfig, ClientJsonRpcMessage, ClientRequest, DiscoverRequest, DiscoverRequestParams,
    ErrorCode, InitializeRequest, JsonRpcMessage, NumberOrString, ProtocolVersion, RequestId,
    RequestMetaObject, ServerResult,
};
use rmcp::service::{
    ClientInitializeError, ClientLifecycleMode, RoleClient, RxJsonRpcMessage, TxJsonRpcMessage,
};
use rmcp::transport::Transport;

/// The protocol glave speaks first.
const NEWER: ProtocolVersion = ProtocolVersion::V_2026_07_28;
/// How long the probe waits alone before the older handshake is offered beside it, as rmcp's
/// automatic lifecycle waits: a server on the older protocol that never answers the probe still
/// starts that soon.
const PATIENCE: Duration = Duration::from_secs(10);
/// The most messages kept that came before an answer, for rmcp to read once it starts.
const HELD: usize = 64;

/// One of the two requests a probe makes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Asking {
    Discover,
    Initialize,
}

/// What becomes of the answer to one of glave's own requests.
enum Then {
    /// Not answered yet.
    Waiting,
    /// Answered before rmcp asked the same: its answer to that.
    Answered(Box<RxJsonRpcMessage<RoleClient>>),
    /// rmcp asked the same before it was answered: its answer, as the answer to rmcp's.
    Passed(RequestId),
    /// The request that did not decide: its answer is dropped.
    Dropped,
}

/// One of glave's own requests, until its answer comes.
struct Asked {
    asking: Asking,
    id: RequestId,
    then: Then,
}

/// `inner`, after its probe: rmcp's first request answered with the answer already in, and
/// the answer to the request that did not decide left out.
pub(crate) struct Probed<T> {
    inner: T,
    /// What came during the probe that answers neither request, for rmcp first.
    held: VecDeque<RxJsonRpcMessage<RoleClient>>,
    asked: Vec<Asked>,
}

/// Asks the server at `inner` which protocol it speaks, as `info` says glave is, and gives the
/// transport back with the lifecycle rmcp starts it in. Waits as long as the server takes to
/// say: the caller's limit bounds it.
pub(crate) async fn probe<T: Transport<RoleClient> + 'static>(
    inner: T,
    info: &ClientConfig,
) -> Result<(Probed<T>, ClientLifecycleMode), Box<ClientInitializeError>> {
    let mut probed = Probed {
        inner,
        held: VecDeque::new(),
        asked: Vec::new(),
    };
    probed.ask(Asking::Discover, discover(info)).await?;
    let patience = tokio::time::sleep(PATIENCE);
    tokio::pin!(patience);
    let mut offered = false;
    loop {
        let message = if offered {
            probed.inner.receive().await
        } else {
            tokio::select! {
                message = probed.inner.receive() => message,
                () = &mut patience => {
                    offered = true;
                    probed.ask(Asking::Initialize, initialize(info)).await?;
                    continue;
                }
            }
        };
        let Some(message) = message else {
            return Err(Box::new(ClientInitializeError::ConnectionClosed(
                "the server closed the connection before saying which protocol it speaks".into(),
            )));
        };
        let Some(asking) = probed.answers(&message) else {
            if probed.held.len() < HELD {
                probed.held.push_back(message);
            }
            continue;
        };
        if let Some(lifecycle) = probed.decide(asking, message) {
            return Ok((probed, lifecycle));
        }
    }
}

impl<T: Transport<RoleClient> + 'static> Probed<T> {
    /// Sends `request` as glave's own.
    async fn ask(
        &mut self,
        asking: Asking,
        request: ClientRequest,
    ) -> Result<(), Box<ClientInitializeError>> {
        let id = NumberOrString::String(Arc::from(match asking {
            Asking::Discover => "glave-discover",
            Asking::Initialize => "glave-initialize",
        }));
        self.asked.push(Asked {
            asking,
            id: id.clone(),
            then: Then::Waiting,
        });
        let sent = self.inner.send(ClientJsonRpcMessage::request(request, id));
        sent.await.map_err(|error| {
            Box::new(ClientInitializeError::transport::<T>(
                error,
                "send the first request",
            ))
        })
    }
}

impl<T> Probed<T> {
    /// Which of glave's requests waiting for an answer `message` answers, if any. An error with
    /// no id answers the probe, the first request, as rmcp takes it.
    fn answers(&self, message: &RxJsonRpcMessage<RoleClient>) -> Option<Asking> {
        let id = match message {
            JsonRpcMessage::Error(error) if error.id.is_none() => None,
            _ => Some(answered(message)?),
        };
        self.asked
            .iter()
            .find(|asked| {
                matches!(asked.then, Then::Waiting) && id.is_none_or(|id| is(id, &asked.id))
            })
            .map(|asked| asked.asking)
    }

    /// What the answer `message` to `asking` says, as the lifecycle rmcp starts in once it
    /// settles which protocol the server speaks.
    fn decide(
        &mut self,
        asking: Asking,
        message: RxJsonRpcMessage<RoleClient>,
    ) -> Option<ClientLifecycleMode> {
        let newer = ClientLifecycleMode::Discover {
            preferred_versions: vec![NEWER],
        };
        match (asking, says(asking, &message)) {
            (Asking::Discover, Some(Era::Newer)) => {
                self.then(Asking::Discover, Then::Answered(Box::new(message)));
                self.then(Asking::Initialize, Then::Dropped);
                Some(newer)
            }
            // The handshake, if offered, has its answer to come, for rmcp's.
            (Asking::Discover, _) => {
                self.then(Asking::Discover, Then::Dropped);
                Some(ClientLifecycleMode::Initialize)
            }
            (Asking::Initialize, Some(Era::Older)) => {
                self.then(Asking::Initialize, Then::Answered(Box::new(message)));
                self.then(Asking::Discover, Then::Dropped);
                Some(ClientLifecycleMode::Initialize)
            }
            // Refused, so not the older protocol after all: the probe's answer decides.
            (Asking::Initialize, _) => {
                self.then(Asking::Initialize, Then::Dropped);
                None
            }
        }
    }

    /// Sets what becomes of the answer to `asking`, while it waits.
    fn then(&mut self, asking: Asking, then: Then) {
        if let Some(asked) = self
            .asked
            .iter_mut()
            .find(|asked| asked.asking == asking && matches!(asked.then, Then::Waiting))
        {
            asked.then = then;
        }
    }
}

/// Which protocol a server speaks, by an answer.
enum Era {
    Newer,
    Older,
}

/// What `message`, the answer to `asking`, says the server speaks, when it says. An error
/// MCP 2026-07-28 defines, such as a version it does not speak, is the newer protocol's, and
/// rmcp goes on from it; any other answer to the probe is the older protocol's.
fn says(asking: Asking, message: &RxJsonRpcMessage<RoleClient>) -> Option<Era> {
    match (asking, message) {
        (Asking::Discover, JsonRpcMessage::Response(response)) => {
            matches!(response.result, ServerResult::DiscoverResult(_)).then_some(Era::Newer)
        }
        (Asking::Discover, JsonRpcMessage::Error(error)) => matches!(
            error.error.code,
            ErrorCode::UNSUPPORTED_PROTOCOL_VERSION
                | ErrorCode::MISSING_REQUIRED_CLIENT_CAPABILITY
                | ErrorCode::HEADER_MISMATCH
        )
        .then_some(Era::Newer),
        (Asking::Initialize, JsonRpcMessage::Response(response)) => {
            matches!(response.result, ServerResult::InitializeResult(_)).then_some(Era::Older)
        }
        _ => None,
    }
}

/// The probe, `server/discover` on the newer protocol.
fn discover(info: &ClientConfig) -> ClientRequest {
    let mut discover = DiscoverRequest::new(DiscoverRequestParams {});
    discover
        .extensions
        .insert(RequestMetaObject::with_client_context(
            NEWER,
            info.client_info.clone(),
            info.capabilities.clone(),
        ));
    ClientRequest::DiscoverRequest(discover)
}

/// The older protocol's handshake.
fn initialize(info: &ClientConfig) -> ClientRequest {
    let mut params = info.clone();
    older(&mut params);
    ClientRequest::InitializeRequest(InitializeRequest::new(params))
}

/// `params` for the older protocol's handshake: its newest version, and no forms. A server on
/// it asks with the call held open, which glave never lets it: told so, it can do without
/// rather than be declined.
fn older(params: &mut ClientConfig) {
    params.protocol_version = ProtocolVersion::LATEST_WITH_INITIALIZE;
    params.capabilities.elicitation = None;
}

/// The id of the request `message` answers, when it answers one and names it.
fn answered(message: &RxJsonRpcMessage<RoleClient>) -> Option<&RequestId> {
    match message {
        JsonRpcMessage::Response(response) => Some(&response.id),
        JsonRpcMessage::Error(error) => error.id.as_ref(),
        JsonRpcMessage::Request(_) | JsonRpcMessage::Notification(_) => None,
    }
}

/// Whether `id` answers the request `of`, a number sent back as a string too, as rmcp matches
/// them.
fn is(id: &RequestId, of: &RequestId) -> bool {
    id == of
        || matches!((of, id), (NumberOrString::Number(of), NumberOrString::String(id))
            if id.parse() == Ok(*of))
}

/// `message`, an answer, as the answer to `id`.
fn answering(
    mut message: RxJsonRpcMessage<RoleClient>,
    id: RequestId,
) -> RxJsonRpcMessage<RoleClient> {
    match &mut message {
        JsonRpcMessage::Response(response) => response.id = id,
        JsonRpcMessage::Error(error) => error.id = Some(id),
        JsonRpcMessage::Request(_) | JsonRpcMessage::Notification(_) => {}
    }
    message
}

#[deny(clippy::missing_trait_methods)]
impl<T: Transport<RoleClient>> Transport<RoleClient> for Probed<T> {
    type Error = T::Error;

    fn name() -> std::borrow::Cow<'static, str> {
        T::name()
    }

    fn send(
        &mut self,
        mut item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        let mut answered = false;
        if let JsonRpcMessage::Request(request) = &mut item {
            let asking = match &mut request.request {
                ClientRequest::DiscoverRequest(_) => Some(Asking::Discover),
                ClientRequest::InitializeRequest(initialize) => {
                    older(&mut initialize.params);
                    Some(Asking::Initialize)
                }
                _ => None,
            };
            let mine = self.asked.iter_mut().find(|asked| {
                Some(asked.asking) == asking
                    && matches!(asked.then, Then::Waiting | Then::Answered(_))
            });
            if let Some(asked) = mine {
                answered = true;
                let id = request.id.clone();
                match std::mem::replace(&mut asked.then, Then::Dropped) {
                    Then::Answered(answer) => self.held.push_back(answering(*answer, id)),
                    _ => asked.then = Then::Passed(id),
                }
            }
        }
        let sent = (!answered).then(|| self.inner.send(item));
        async move {
            match sent {
                Some(sent) => sent.await,
                None => Ok(()),
            }
        }
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        loop {
            if let Some(message) = self.held.pop_front() {
                return Some(message);
            }
            let message = self.inner.receive().await?;
            let Some(id) = answered(&message) else {
                return Some(message);
            };
            let Some(asked) = self.asked.extract_if(.., |asked| is(id, &asked.id)).next() else {
                return Some(message);
            };
            if let Then::Passed(theirs) = asked.then {
                return Some(answering(message, theirs));
            }
        }
    }

    fn close(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.inner.close()
    }
}
