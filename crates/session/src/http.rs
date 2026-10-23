//! The session API over HTTP (`specs/session.md`, "Transports"). A client `POST`s one JSON-RPC
//! request to `/` with an OIDC access token, or from this computer with the server's local
//! token. The answer is JSON, or for `turn/start`, `goal/start`, `thread/watch` and
//! `thread/list/watch` a `text/event-stream` of notifications ending with the response (`body`):
//! a watch's events carry their `seq` as their id, a list's summaries their `updated_at`, and a
//! comment keeps a quiet stream open. The method may
//! also be named in the `Glave-Method` header, for proxies and logs; when it is, it must match.
//! Every request needs a valid sign-in, but `server/discover`, from this computer
//! `signin/redeem`, and anywhere but where the server has no public origin the passkey methods
//! that sign a browser in: without one the answer is HTTP 401 with error `-32013`. Which way
//! a request came in, its door, is the listener's and the request's (`hosts`). Served with a
//! [`Site`], any `GET` is the web app instead (`site`), with no sign-in: the app signs in to
//! the API itself. A foreign `Host` on a loopback listener, or a page of another origin, is
//! refused (`hosts`). Beside plain HTTP, the server may serve HTTPS of its own (`listen`).

mod body;
mod hosts;
mod listen;
mod site;

pub use self::listen::Listeners;
pub use self::site::{Files, NoSite, Site};

use std::convert::Infallible;
use std::future::{Future, poll_fn};
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
use tokio::task::JoinSet;

use self::body::Answer;
use self::hosts::door;
use self::listen::{accept, connected};
use crate::dispatch::Answered;
use crate::protocol::{ApiError, Request, code};
use crate::rpc::{Id, parse, respond};
use crate::server::Server;
use crate::signin::{Door, SignIn, SignInError};
use crate::streamed::Streamed;

/// The largest request body accepted, in bytes.
const MAX_BODY: usize = 16 * 1024 * 1024;

/// The largest request body accepted without a sign-in, in bytes: room for a code, or a
/// passkey's answer.
const MAX_OPEN_BODY: usize = 16 * 1024;

/// Connections served at once.
const MAX_CONNECTIONS: usize = 1_024;

/// How long a client may take to send a request's headers.
const HEADER_TIMEOUT: Duration = Duration::from_secs(30);

/// The header that names the method.
const METHOD_HEADER: &str = "glave-method";

/// Serves `server` over HTTP on `listeners`, each request signed in through `signin`, until
/// `stop` completes. Then it cancels running turns and closes every connection. With `site`,
/// the web app too: every `GET` is one of its files. Browsers the server signs in sign in
/// through `signin` too.
pub async fn serve<P, L, B, S>(
    server: Server<P, L, B>,
    signin: SignIn,
    site: Option<Site>,
    listeners: Listeners,
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
    // Browsers on this computer make and use passkeys at `localhost`, where only this server
    // answers, unless the server is reached at a public origin, its relying party then.
    if server.inner.origin.is_none()
        && let (Some(browsers), Some(port)) = (&server.inner.browsers, listeners.localhost())
        && let Err(error) = browsers.reached_here(port)
    {
        tracing::warn!(%error, "browsers here cannot sign in with passkeys");
    }
    let all = listeners.listening(server.inner.origin.as_ref())?;
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
                accepted = accept(&all) => accepted,
            }
        };
        // A connection that failed to open is that client's problem, not the server's.
        let (Ok((stream, peer)), listening) = accepted else {
            continue;
        };
        let peer = peer.ip();
        let (server, signin, site) = (server.clone(), Arc::clone(&signin), Arc::clone(&site));
        let (hosts, tls) = (Arc::clone(&listening.names), listening.handshakes());
        connections.spawn(async move {
            let service = service_fn(move |request: HttpRequest<Incoming>| {
                let (server, signin, site) =
                    (server.clone(), Arc::clone(&signin), Arc::clone(&site));
                let names = hosts.as_ref().as_ref();
                let foreign = names.is_some_and(|hosts| !hosts.allow(request.headers()));
                let door = door(names, request.headers(), peer);
                async move {
                    if foreign {
                        return Ok::<_, Infallible>(plain(StatusCode::FORBIDDEN));
                    }
                    let answered = match site.as_ref() {
                        Some(site) if is_read(request.method()) => {
                            site.answer(request.uri().path()).await
                        }
                        _ => answer(&server, &signin, door, request).await,
                    };
                    Ok::<_, Infallible>(answered)
                }
            });
            let Some(stream) = connected(tls, stream).await else {
                return;
            };
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

/// Answers one HTTP request, which came in at `door`.
async fn answer<P, L, B>(
    server: &Server<P, L, B>,
    signin: &SignIn,
    door: Door,
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
    // The languages the client reads, for the server's own sentences when the caller chose none.
    let asked = request
        .headers()
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let caller = match &token {
        Some(token) => match signin.verify_from(token, door).await {
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
        None => match open(&request, door) {
            // Neither reads who calls.
            Ok(()) => glave_ports::Caller::default(),
            Err(refused) => return unauthorized(&refused),
        },
    };
    dispatch(server, &caller, id, request, asked.as_deref()).await
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

/// Whether `request` may be answered without a sign-in, coming in at `door`: what the server
/// is; trading a link's code for a session, from this computer; and signing a browser in with
/// a passkey, or making the first with a setup link's code, but where the server has no public
/// origin.
fn open(request: &Request, door: Door) -> Result<(), SignInError> {
    match request {
        Request::Discover {} => Ok(()),
        Request::SigninRedeem { .. } if door == Door::Here => Ok(()),
        Request::SigninRedeem { .. } => Err(SignInError::NotLoopback),
        Request::PasskeyCreateOptions { .. }
        | Request::PasskeyCreate { .. }
        | Request::PasskeyGetOptions {}
        | Request::PasskeyGet { .. }
            if door != Door::Elsewhere =>
        {
            Ok(())
        }
        _ => Err(SignInError::Missing),
    }
}

/// Runs a request for a signed-in caller, who reads the languages `asked` (`Accept-Language`).
async fn dispatch<P, L, B>(
    server: &Server<P, L, B>,
    caller: &glave_ports::Caller,
    id: Option<Id>,
    request: Request,
    asked: Option<&str>,
) -> Response<Answer>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    let streamed = match server.call(caller, request).await {
        Answered::Reply(reply) => match id {
            Some(id) => {
                let reply = server.in_language(caller, asked, reply).await;
                return json(StatusCode::OK, respond(Some(&id), reply.as_ref()));
            }
            None => return accepted(),
        },
        Answered::Stream(turn) => Streamed::Turn(turn),
        Answered::Watch(watch) => Streamed::Watch(watch),
        Answered::List(watch) => Streamed::List(watch),
    };
    match id {
        Some(id) => {
            let language = server.language_for(caller, asked).await;
            events(Answer::stream(streamed, Some(id), language))
        }
        // A notification is never answered: a turn goes on without anyone following, and a
        // watch ends.
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
