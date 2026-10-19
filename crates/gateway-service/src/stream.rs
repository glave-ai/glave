//! One model call through the service, and its answer as a stream of server-sent events. The
//! call runs inside the response body: it goes on only while the client reads, so a slow
//! client slows the model's stream rather than filling memory, and a client that leaves
//! cancels the call. Until the first fragment arrives nothing is sent, so a call refused before
//! it starts (a budget, a refusal by the provider) is answered with its own HTTP status.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};

use bytes::Bytes;
use glave_event::{ModelResponded, PrincipalId};
use glave_gateway::{BudgetKey, Target};
use glave_ports::{Completion, Delta, EventLog, ModelRequest};
use hyper::body::{Body, Frame, SizeHint};

use crate::errors::{Kind, Refusal};
use crate::service::Service;

/// How a surface writes an answer as events.
pub(crate) trait Render: Send + 'static {
    /// The events for one live fragment.
    fn delta(&mut self, delta: Delta) -> Vec<Bytes>;
    /// The events that end a finished answer.
    fn finish(&mut self, completion: &Completion) -> Vec<Bytes>;
    /// The events that end an answer that failed after it began.
    fn fail(&mut self, refusal: &Refusal) -> Vec<Bytes>;
}

/// The renderer and the events it wrote that are not sent yet.
pub(crate) struct Pending<R> {
    render: R,
    queue: VecDeque<Bytes>,
}

type Shared<R> = Arc<Mutex<Pending<R>>>;

fn lock<R>(shared: &Shared<R>) -> MutexGuard<'_, Pending<R>> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A call under way.
pub(crate) type Call = Pin<Box<dyn Future<Output = Result<Completion, Refusal>> + Send>>;

/// Runs `request` for `person` on `targets`, live fragments going to `shared` when given, and
/// records the answer in their stream. A budget that cannot be read back refuses the call: a
/// budget is never guessed.
fn call<L, R>(
    service: Arc<Service<L>>,
    person: PrincipalId,
    targets: Vec<Target>,
    request: ModelRequest,
    shared: Option<Shared<R>>,
) -> Call
where
    L: EventLog + 'static,
    R: Render,
{
    Box::pin(async move {
        let day = service.meter.today();
        if let Err(error) = service.meter.seed(&service.gateway, person, day).await {
            let message = format!("the gateway could not read back what you spent today: {error}");
            return Err(Refusal::new(Kind::Upstream, service.redact(&message)));
        }
        let mut on_delta = |delta: Delta| {
            if let Some(shared) = &shared {
                let mut pending = lock(shared);
                let events = pending.render.delta(delta);
                pending.queue.extend(events);
            }
        };
        let caller = BudgetKey {
            person,
            period: day,
        };
        let completion = service
            .gateway
            .complete_for(caller, Some(&targets), &request, &mut on_delta)
            .await
            .map_err(|error| {
                let mut refusal = Refusal::of(&error);
                refusal.message = service.redact(&refusal.message);
                refusal
            })?;
        if let Err(error) = service.meter.record(person, &responded(&completion)).await {
            service.notice(&format!(
                "a model call by {person} was not recorded: {error}"
            ));
        }
        Ok(completion)
    })
}

/// The content-free record of a finished call.
fn responded(completion: &Completion) -> ModelResponded {
    let served = completion.served.as_ref();
    ModelResponded {
        model: completion.model.clone(),
        response_id: completion.response_id.clone(),
        stop: completion.stop,
        provider_stop: completion.provider_stop.clone(),
        usage: completion.usage.clone(),
        cost: completion.cost.clone(),
        route: served.map(|served| served.route.clone()),
        target: served.map(|served| served.target.clone()),
        reasoning_stripped: served.is_some_and(|served| served.reasoning_stripped),
    }
}

/// Runs a call whole, for a client that did not ask for a stream.
pub(crate) async fn whole<L: EventLog + 'static>(
    service: Arc<Service<L>>,
    person: PrincipalId,
    targets: Vec<Target>,
    request: ModelRequest,
) -> Result<Completion, Refusal> {
    call::<L, Silent>(service, person, targets, request, None).await
}

/// A renderer for a call nobody streams.
struct Silent;

impl Render for Silent {
    fn delta(&mut self, _: Delta) -> Vec<Bytes> {
        Vec::new()
    }
    fn finish(&mut self, _: &Completion) -> Vec<Bytes> {
        Vec::new()
    }
    fn fail(&mut self, _: &Refusal) -> Vec<Bytes> {
        Vec::new()
    }
}

/// A call's answer as server-sent events, the call running as they are read. The call sits in
/// a mutex only so the body may be shared between threads, as hyper's boxed bodies must; it is
/// reached through `get_mut`, never locked.
pub(crate) struct Streaming<R> {
    shared: Shared<R>,
    call: Mutex<Option<Call>>,
}

fn running(call: &mut Mutex<Option<Call>>) -> &mut Option<Call> {
    call.get_mut().unwrap_or_else(PoisonError::into_inner)
}

impl<R: Render> Streaming<R> {
    /// Starts `request` for `person` on `targets`, written by `render`, and waits until it
    /// has something to send: then the stream to send. A call that fails before saying
    /// anything comes back as its refusal, to answer with its own status.
    pub(crate) async fn start<L: EventLog + 'static>(
        service: Arc<Service<L>>,
        person: PrincipalId,
        targets: Vec<Target>,
        request: ModelRequest,
        render: R,
    ) -> Result<Self, Refusal> {
        let shared = Arc::new(Mutex::new(Pending {
            render,
            queue: VecDeque::new(),
        }));
        let call = call(service, person, targets, request, Some(Arc::clone(&shared)));
        let mut streaming = Self {
            shared,
            call: Mutex::new(Some(call)),
        };
        poll_fn(|cx| streaming.first(cx)).await?;
        Ok(streaming)
    }

    /// Ready once there is something to send, or the call failed with nothing sent.
    fn first(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Refusal>> {
        let Some(call) = running(&mut self.call).as_mut() else {
            return Poll::Ready(Ok(()));
        };
        match call.as_mut().poll(cx) {
            Poll::Ready(Err(refusal)) if lock(&self.shared).queue.is_empty() => {
                Poll::Ready(Err(refusal))
            }
            Poll::Ready(outcome) => {
                *running(&mut self.call) = None;
                self.end(&outcome);
                Poll::Ready(Ok(()))
            }
            Poll::Pending if lock(&self.shared).queue.is_empty() => Poll::Pending,
            Poll::Pending => Poll::Ready(Ok(())),
        }
    }

    /// Queues the events that end the answer.
    fn end(&self, outcome: &Result<Completion, Refusal>) {
        let mut pending = lock(&self.shared);
        let events = match outcome {
            Ok(completion) => pending.render.finish(completion),
            Err(refusal) => pending.render.fail(refusal),
        };
        pending.queue.extend(events);
    }
}

impl<R: Render> Body for Streaming<R> {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        let this = self.get_mut();
        loop {
            if let Some(event) = lock(&this.shared).queue.pop_front() {
                return Poll::Ready(Some(Ok(Frame::data(event))));
            }
            let Some(call) = running(&mut this.call).as_mut() else {
                return Poll::Ready(None);
            };
            match call.as_mut().poll(cx) {
                Poll::Ready(outcome) => {
                    *running(&mut this.call) = None;
                    this.end(&outcome);
                }
                // A fragment that arrived during the poll is sent at once.
                Poll::Pending if lock(&this.shared).queue.is_empty() => return Poll::Pending,
                Poll::Pending => {}
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        let ended = self
            .call
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_none();
        ended && lock(&self.shared).queue.is_empty()
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::default()
    }
}

/// One server-sent event: `event: <name>` when named, then its data.
pub(crate) fn event(name: Option<&str>, data: &impl serde::Serialize) -> Bytes {
    let data = serde_json::to_string(data).unwrap_or_default();
    match name {
        Some(name) => Bytes::from(format!("event: {name}\ndata: {data}\n\n")),
        None => Bytes::from(format!("data: {data}\n\n")),
    }
}
