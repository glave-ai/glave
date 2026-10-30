//! The answer to a protocol probe rmcp gave up on, dropped. rmcp's automatic lifecycle asks a
//! server `server/discover` first and, with no answer within 10 s, falls back to the older
//! `initialize` handshake on the same transport; the probe's answer arriving after that fails
//! the start ("conflict initialized response id", or "uncorrelated error response" for an
//! error). So a server slow to answer its first request, such as one whose token is renewed
//! first or a program still starting, never started. Once `initialize` follows a probe still
//! unanswered, that probe's answer is dropped, and the server starts within glave's own limit
//! ([`crate::Limits`]).

use std::borrow::Cow;

use rmcp::model::{ClientRequest, JsonRpcMessage, NumberOrString, RequestId};
use rmcp::service::{RoleClient, RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::Transport;

/// `inner`, less any answer to a probe given up on.
pub(crate) struct Probed<T> {
    inner: T,
    /// The probe sent, until it is answered.
    asking: Option<RequestId>,
    /// The probe given up on: `initialize` was sent before it was answered.
    abandoned: Option<RequestId>,
}

impl<T> Probed<T> {
    pub(crate) fn new(inner: T) -> Self {
        Self {
            inner,
            asking: None,
            abandoned: None,
        }
    }
}

/// The id of the request `message` answers, when it answers one.
fn answered(message: &RxJsonRpcMessage<RoleClient>) -> Option<&RequestId> {
    match message {
        JsonRpcMessage::Response(response) => Some(&response.id),
        JsonRpcMessage::Error(error) => error.id.as_ref(),
        JsonRpcMessage::Request(_) | JsonRpcMessage::Notification(_) => None,
    }
}

/// Whether `id` answers the request `of`, a number sent back as a string too, as rmcp matches
/// them.
fn is(id: Option<&RequestId>, of: Option<&RequestId>) -> bool {
    id.zip(of).is_some_and(|(id, of)| {
        id == of
            || matches!((of, id), (NumberOrString::Number(of), NumberOrString::String(id))
                if id.parse() == Ok(*of))
    })
}

#[deny(clippy::missing_trait_methods)]
impl<T: Transport<RoleClient>> Transport<RoleClient> for Probed<T> {
    type Error = T::Error;

    fn name() -> Cow<'static, str> {
        T::name()
    }

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        if let JsonRpcMessage::Request(request) = &item {
            match request.request {
                ClientRequest::DiscoverRequest(_) => self.asking = Some(request.id.clone()),
                ClientRequest::InitializeRequest(_) => self.abandoned = self.asking.take(),
                _ => {}
            }
        }
        self.inner.send(item)
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        loop {
            let message = self.inner.receive().await?;
            let id = answered(&message);
            if is(id, self.abandoned.as_ref()) {
                self.abandoned = None;
                continue;
            }
            if is(id, self.asking.as_ref()) {
                self.asking = None;
            }
            return Some(message);
        }
    }

    fn close(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.inner.close()
    }
}
