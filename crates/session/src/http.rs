//! The session API over HTTP (`specs/session.md`, "Transports"). A client `POST`s one JSON-RPC
//! request to `/` with an OIDC access token, or from this computer with the server's local
//! token. The answer is JSON, or for `turn/start` a
//! `text/event-stream` of the turn's notifications ending with its response. The method may
//! also be named in the `Glave-Method` header, for proxies and logs; when it is, it must match.
//! Every request needs a valid sign-in, but `server/discover` and, from this computer,
//! `signin/redeem`: without one the answer is HTTP 401 with error `-32013`. Served with a
//! [`Site`], any `GET` is the web app instead (`site`), with no sign-in: the app signs in to
//! the API itself.

mod body;
mod site;

pub use self::site::{Files, NoSite, Site};

use std::convert::Infallible;
use std::future::{Future, poll_fn};
use std::net::IpAddr;
use std::pin::{Pin, pin};
use std::sync::Arc;
use std::time::Duration;

use glave_ports::{BlobStore, EventLog, ModelProvider};
use hyper::body::{Body as _, Incoming};
use hyper::header::{self, HeaderMap, HeaderValue};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request as HttpRequest, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use self::body::Answer;
use crate::dispatch::Answered;
use crate::protocol::{ApiError, Request, code};
use crate::rpc::{Id, parse, respond};
use crate::server::Server;
use crate::signin::{SignIn, SignInError, is_loopback};

/// The largest request body accepted, in bytes.
const MAX_BODY: usize = 16 * 1024 * 1024;

/// The largest request body accepted without a sign-in, in bytes: room for a code.
const MAX_OPEN_BODY: usize = 4 * 1024;

/// Connections served at once.
const MAX_CONNECTIONS: usize = 1_024;

/// How long a client may take to send a request's headers.
const HEADER_TIMEOUT: Duration = Duration::from_secs(30);

/// The header that names the method.
const METHOD_HEADER: &str = "glave-method";

/// Serves `server` over HTTP on `listener`, each request signed in through `signin`, until
/// `stop` completes. Then it cancels running turns and closes every connection. With `site`,
/// the web app too: every `GET` is one of its files. Browsers the server signs in sign in
/// through `signin` too.
pub async fn serve<P, L, B, S>(
    server: Server<P, L, B>,
    signin: SignIn,
    site: Option<Site>,
    listener: TcpListener,
    stop: S,
) -> std::io::Result<()>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
    S: Future<Output = ()>,
{
    let signin = Arc::new(match server.inner.browsers.clone() {
        Some(browsers) => signin.with_browsers(browsers),
        None => signin,
    });
    let site = Arc::new(site);
    let mut connections = JoinSet::new();
    let mut stop = pin!(stop);
    loop {
        while connections.try_join_next().is_some() {}
        // At the cap, the next connection waits until one closes.
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
        // A connection that failed to open is that client's problem, not the server's.
        let Ok((stream, peer)) = accepted else {
            continue;
        };
        let peer = peer.ip();
        let (server, signin, site) = (server.clone(), Arc::clone(&signin), Arc::clone(&site));
        connections.spawn(async move {
            let service = service_fn(move |request| {
                let (server, signin, site) =
                    (server.clone(), Arc::clone(&signin), Arc::clone(&site));
                async move {
                    let answered = match site.as_ref() {
                        Some(site) if is_read(request.method()) => {
                            site.answer(request.uri().path()).await
                        }
                        _ => answer(&server, &signin, peer, request).await,
                    };
                    Ok::<_, Infallible>(answered)
                }
            });
            // A client that goes away mid-request ends only its own connection.
            let _ = http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(HEADER_TIMEOUT)
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
    }
    server.shutdown().await;
    connections.abort_all();
    while connections.join_next().await.is_some() {}
    Ok(())
}

/// Answers one HTTP request.
async fn answer<P, L, B>(
    server: &Server<P, L, B>,
    signin: &SignIn,
    peer: IpAddr,
    request: HttpRequest<Incoming>,
) -> Response<Answer>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    if request.uri().path() != "/" {
        return plain(StatusCode::NOT_FOUND);
    }
    if request.method() != Method::POST {
        let mut refused = plain(StatusCode::METHOD_NOT_ALLOWED);
        refused
            .headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static("POST"));
        return refused;
    }
    let token = bearer(request.headers()).map(str::to_owned);
    let caller = match &token {
        Some(token) => match signin.verify_from(token, peer).await {
            Ok(caller) => Some(caller),
            Err(refused) => return unauthorized(&refused),
        },
        None => None,
    };
    let most = if caller.is_some() {
        MAX_BODY
    } else {
        MAX_OPEN_BODY
    };
    let (id, request) = match message(request, most).await {
        Ok(message) => message,
        // Without a sign-in, nothing is said about what was sent.
        Err(_) if caller.is_none() => return unauthorized(&SignInError::Missing),
        Err(failed) => return json(StatusCode::OK, failed),
    };
    let caller = match caller {
        Some(caller) => caller,
        None => match open(&request, peer) {
            // Neither reads who calls.
            Ok(()) => crate::Caller::default(),
            Err(refused) => return unauthorized(&refused),
        },
    };
    dispatch(server, &caller, id, request).await
}

/// The JSON-RPC request `request` carries, of at most `most` bytes, or the error response
/// saying why it is none. The `Glave-Method` header, when sent, must name its method.
async fn message(
    request: HttpRequest<Incoming>,
    most: usize,
) -> Result<(Option<Id>, Request), String> {
    let named = request
        .headers()
        .get(METHOD_HEADER)
        .map(|value| value.to_str().unwrap_or("").to_owned());
    let line = read(request.into_body(), most)
        .await
        .map_err(|failed| respond(None, Err(&failed)))?;
    let (id, request) = parse(&line).map_err(|(id, failed)| respond(id.as_ref(), Err(&failed)))?;
    if let Some(named) = named
        && named != request.method()
    {
        let failed = ApiError::new(
            code::INVALID_REQUEST,
            format!(
                "the Glave-Method header says {named:?}, the request {:?}",
                request.method()
            ),
        );
        return Err(respond(id.as_ref(), Err(&failed)));
    }
    Ok((id, request))
}

/// Whether `request` may be answered without a sign-in, from `peer`: what the server is, and
/// trading a link's code for a session from this computer.
fn open(request: &Request, peer: IpAddr) -> Result<(), SignInError> {
    match request {
        Request::Discover {} => Ok(()),
        Request::SigninRedeem { .. } if is_loopback(peer) => Ok(()),
        Request::SigninRedeem { .. } => Err(SignInError::NotLoopback),
        _ => Err(SignInError::Missing),
    }
}

/// Runs a request for a signed-in caller.
async fn dispatch<P, L, B>(
    server: &Server<P, L, B>,
    caller: &crate::Caller,
    id: Option<Id>,
    request: Request,
) -> Response<Answer>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    let answered = match server.call(caller, request).await {
        // A notification is never answered; the turn goes on without anyone following.
        Answered::Stream(_) if id.is_none() => return accepted(),
        Answered::Stream(stream) => return events(Answer::turn(stream, id)),
        Answered::Reply(reply) => reply,
    };
    match id {
        Some(id) => json(StatusCode::OK, respond(Some(&id), answered.as_ref())),
        None => accepted(),
    }
}

/// Whether `method` reads a page rather than calling the API.
fn is_read(method: &Method) -> bool {
    *method == Method::GET || *method == Method::HEAD
}

/// The token in `Authorization: Bearer <token>`, if there is one.
fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

/// The request body as text, up to `most` bytes.
async fn read(body: Incoming, most: usize) -> Result<String, ApiError> {
    let failed = |message: String| ApiError::new(code::PARSE, message);
    let mut body = pin!(body);
    let mut bytes = Vec::new();
    while let Some(frame) = poll_fn(|cx| Pin::as_mut(&mut body).poll_frame(cx)).await {
        let frame = frame.map_err(|e| failed(format!("could not read the request: {e}")))?;
        if let Ok(data) = frame.into_data() {
            if bytes.len().saturating_add(data.len()) > most {
                return Err(failed(format!("a request is longer than {most} bytes")));
            }
            bytes.extend_from_slice(&data);
        }
    }
    String::from_utf8(bytes).map_err(|e| failed(format!("the request is not UTF-8: {e}")))
}

fn plain(status: StatusCode) -> Response<Answer> {
    let mut response = Response::new(Answer::empty());
    *response.status_mut() = status;
    response
}

fn accepted() -> Response<Answer> {
    plain(StatusCode::ACCEPTED)
}

fn json(status: StatusCode, message: String) -> Response<Answer> {
    let mut response = Response::new(Answer::once(message));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}

fn events(body: Answer) -> Response<Answer> {
    let mut response = Response::new(body);
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// HTTP 401 with error `-32013`, saying what is wrong with the sign-in but never the token.
fn unauthorized(refused: &SignInError) -> Response<Answer> {
    let failed = ApiError::new(code::NO_SIGN_IN, refused.to_string());
    let mut response = json(StatusCode::UNAUTHORIZED, respond(None, Err(&failed)));
    response
        .headers_mut()
        .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_comes_from_a_bearer_authorization_only() {
        let with = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(header::AUTHORIZATION, HeaderValue::from_str(value).unwrap());
            bearer(&headers).map(str::to_owned)
        };
        assert_eq!(with("Bearer abc.def.ghi").as_deref(), Some("abc.def.ghi"));
        assert_eq!(with("bearer abc").as_deref(), Some("abc"));
        assert_eq!(with("Basic dXNlcjpwYXNz"), None);
        assert_eq!(with("Bearer "), None);
        assert_eq!(bearer(&HeaderMap::new()), None);
    }
}
