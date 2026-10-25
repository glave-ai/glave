//! The service over HTTP. `POST /v1/chat/completions` and `POST /v1/messages` take model
//! calls; `/mcp` is the tool catalog over MCP's Streamable HTTP. Every request needs a sign-in:
//! an OIDC access token as `Authorization: Bearer <token>`, or in `x-api-key`, where Anthropic's
//! SDKs put their key. Without one the answer is 401, before the body is read.

use std::convert::Infallible;
use std::future::{Future, poll_fn};
use std::pin::{Pin, pin};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use glave_ports::EventLog;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt as _, Full};
use hyper::body::{Body as _, Incoming};
use hyper::header::{self, HeaderMap, HeaderValue};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use crate::catalog::Catalog;
use crate::errors::{Kind, Refusal};
use crate::service::{Service, Verified};
use crate::stream::{Render, Streaming, whole};
use crate::{anthropic, models, openai};

/// The largest request body accepted, in bytes.
const MAX_BODY: usize = 16 * 1024 * 1024;
/// Connections served at once.
const MAX_CONNECTIONS: usize = 1_024;
/// How long a client may take to send a request's headers.
const HEADER_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a signed-in client may take to send a request's body: the largest, with images,
/// in minutes even on a slow uplink.
const BODY_WITHIN: Duration = Duration::from_mins(10);

type Answer = Response<BoxBody<Bytes, Infallible>>;
type Mcp<L> = StreamableHttpService<Catalog<L>, LocalSessionManager>;

/// Which surface a request came in by, for the shape of its errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Surface {
    OpenAi,
    Anthropic,
}

impl Surface {
    fn error(self, refusal: &Refusal) -> Answer {
        let body = match self {
            Self::OpenAi => refusal.openai(),
            Self::Anthropic => refusal.anthropic(),
        };
        let mut answer = json(refusal.status, body.to_string());
        if refusal.kind == Kind::SignIn {
            answer
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        answer
    }
}

/// Serves `service` on `listener` until `stop` completes, then closes every connection.
pub async fn serve<L, S>(
    service: Arc<Service<L>>,
    listener: TcpListener,
    stop: S,
) -> std::io::Result<()>
where
    L: EventLog + 'static,
    S: Future<Output = ()>,
{
    let catalog = Catalog::new(Arc::clone(&service));
    // Stateless, and any host: every request is signed in, so no session or origin is trusted.
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .disable_allowed_hosts();
    let mcp: Arc<Mcp<L>> = Arc::new(StreamableHttpService::new(
        move || Ok(catalog.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    ));
    let mut connections = JoinSet::new();
    let mut stop = pin!(stop);
    loop {
        while connections.try_join_next().is_some() {}
        let accepted = if connections.len() >= MAX_CONNECTIONS {
            tokio::select! {
                () = &mut stop => break,
                _ = connections.join_next() => continue,
            }
        } else {
            tokio::select! {
                () = &mut stop => break,
                accepted = listener.accept() => accepted,
            }
        };
        let Ok((stream, _)) = accepted else {
            continue;
        };
        let (service, mcp) = (Arc::clone(&service), Arc::clone(&mcp));
        connections.spawn(async move {
            let handler = service_fn(move |request| {
                let (service, mcp) = (Arc::clone(&service), Arc::clone(&mcp));
                async move { Ok::<_, Infallible>(answer(service, &mcp, request).await) }
            });
            // A client that goes away mid-request ends only its own connection.
            let _ = http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(HEADER_TIMEOUT)
                .serve_connection(TokioIo::new(stream), handler)
                .await;
        });
    }
    connections.abort_all();
    while connections.join_next().await.is_some() {}
    Ok(())
}

async fn answer<L: EventLog + 'static>(
    service: Arc<Service<L>>,
    mcp: &Mcp<L>,
    request: Request<Incoming>,
) -> Answer {
    let surface = match request.uri().path() {
        "/v1/chat/completions" => Surface::OpenAi,
        "/v1/messages" => Surface::Anthropic,
        "/mcp" => return catalog(&service, mcp, request).await,
        _ => {
            let missing = Refusal::new(Kind::NotFound, "no such path");
            return Surface::OpenAi.error(&missing);
        }
    };
    if request.method() != Method::POST {
        let mut refused = json(StatusCode::METHOD_NOT_ALLOWED, String::new());
        refused
            .headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static("POST"));
        return refused;
    }
    let verified = match signed_in(&service, request.headers()).await {
        Ok(verified) => verified,
        Err(refusal) => return surface.error(&refusal),
    };
    let body = match read(request.into_body()).await {
        Ok(body) => body,
        Err(refusal) => return surface.error(&refusal),
    };
    let answered = match surface {
        Surface::OpenAi => chat(service, &verified, &body).await,
        Surface::Anthropic => messages(service, &verified, &body).await,
    };
    answered.unwrap_or_else(|refusal| surface.error(&refusal))
}

/// `/v1/chat/completions`.
async fn chat<L: EventLog + 'static>(
    service: Arc<Service<L>>,
    verified: &Verified,
    body: &[u8],
) -> Result<Answer, Refusal> {
    let parsed = openai::parse(body)?;
    let (purpose, targets) = models::resolve(
        &service.gateway,
        service.policy.as_ref(),
        &verified.caller,
        &parsed.model,
    )?;
    let mut request = parsed.request;
    request.purpose = purpose;
    let id = format!("chatcmpl-{}", service.meter.fresh_id()?);
    let created = service.meter.now().as_second();
    if !parsed.stream {
        let completion = whole(service, verified.person, targets, request).await?;
        return Ok(json(
            StatusCode::OK,
            openai::whole(&id, created, &completion),
        ));
    }
    let chunks = openai::Chunks::new(id, created, parsed.model, parsed.usage);
    stream(service, verified, targets, request, chunks).await
}

/// `/v1/messages`.
async fn messages<L: EventLog + 'static>(
    service: Arc<Service<L>>,
    verified: &Verified,
    body: &[u8],
) -> Result<Answer, Refusal> {
    let parsed = anthropic::parse(body)?;
    let (purpose, targets) = models::resolve(
        &service.gateway,
        service.policy.as_ref(),
        &verified.caller,
        &parsed.model,
    )?;
    let mut request = parsed.request;
    request.purpose = purpose;
    let id = format!("msg_{}", service.meter.fresh_id()?);
    if !parsed.stream {
        let completion = whole(service, verified.person, targets, request).await?;
        let message = anthropic::whole(&id, &completion);
        return Ok(json(StatusCode::OK, message.to_string()));
    }
    let events = anthropic::Events::new(id, parsed.model);
    stream(service, verified, targets, request, events).await
}

async fn stream<L: EventLog + 'static, R: Render>(
    service: Arc<Service<L>>,
    verified: &Verified,
    targets: Vec<glave_gateway::Target>,
    request: glave_ports::ModelRequest,
    render: R,
) -> Result<Answer, Refusal> {
    let streaming = Streaming::start(service, verified.person, targets, request, render).await?;
    let mut answer = Response::new(streaming.boxed());
    let headers = answer.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(answer)
}

/// `/mcp`: the catalog, for a signed-in caller, who rides along with the request to it.
async fn catalog<L: EventLog + 'static>(
    service: &Service<L>,
    mcp: &Mcp<L>,
    mut request: Request<Incoming>,
) -> Answer {
    match signed_in(service, request.headers()).await {
        Ok(verified) => {
            request.extensions_mut().insert(verified);
            mcp.handle(request).await
        }
        Err(refusal) => Surface::OpenAi.error(&refusal),
    }
}

/// The caller a request's token proves, or why it proves no one. Never shows the token.
async fn signed_in<L: EventLog>(
    service: &Service<L>,
    headers: &HeaderMap,
) -> Result<Verified, Refusal> {
    let Some(token) = token(headers) else {
        return Err(Refusal::new(
            Kind::SignIn,
            "no sign-in: send an OIDC access token as `Authorization: Bearer <token>`",
        ));
    };
    service
        .verify(token)
        .await
        .map_err(|refused| Refusal::new(Kind::SignIn, refused.to_string()))
}

/// The token in `Authorization: Bearer <token>`, or else in `x-api-key`.
fn token(headers: &HeaderMap) -> Option<&str> {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map(|(_, token)| token.trim());
    bearer
        .or_else(|| {
            headers
                .get("x-api-key")
                .and_then(|value| value.to_str().ok())
                .map(str::trim)
        })
        .filter(|token| !token.is_empty())
}

/// The request body, up to `MAX_BODY` bytes, within [`BODY_WITHIN`].
async fn read(body: Incoming) -> Result<Vec<u8>, Refusal> {
    tokio::time::timeout(BODY_WITHIN, read_all(body))
        .await
        .unwrap_or_else(|_| {
            Err(Refusal::invalid(format!(
                "the request did not arrive whole within {}",
                glave_ports::spoken(BODY_WITHIN)
            )))
        })
}

async fn read_all(body: Incoming) -> Result<Vec<u8>, Refusal> {
    let mut body = pin!(body);
    let mut bytes = Vec::new();
    while let Some(frame) = poll_fn(|cx| Pin::as_mut(&mut body).poll_frame(cx)).await {
        let frame = frame
            .map_err(|error| Refusal::invalid(format!("could not read the request: {error}")))?;
        if let Ok(data) = frame.into_data() {
            if bytes.len().saturating_add(data.len()) > MAX_BODY {
                return Err(Refusal::invalid(format!(
                    "a request is longer than {MAX_BODY} bytes"
                )));
            }
            bytes.extend_from_slice(&data);
        }
    }
    Ok(bytes)
}

fn json(status: StatusCode, body: String) -> Answer {
    let mut answer = Response::new(Full::new(Bytes::from(body)).boxed());
    *answer.status_mut() = status;
    answer.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    answer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_comes_from_a_bearer_authorization_or_an_api_key() {
        let with = |name: &'static str, value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(name, HeaderValue::from_str(value).unwrap());
            token(&headers).map(str::to_owned)
        };
        assert_eq!(
            with("authorization", "Bearer a.b.c").as_deref(),
            Some("a.b.c")
        );
        assert_eq!(with("x-api-key", "a.b.c").as_deref(), Some("a.b.c"));
        assert_eq!(with("authorization", "Basic dXNlcjpwYXNz"), None);
        assert_eq!(with("authorization", "Bearer "), None);
        assert_eq!(token(&HeaderMap::new()), None);
    }
}
