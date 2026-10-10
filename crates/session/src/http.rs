//! The session API over HTTP (`specs/session.md`, "Transports"). A client `POST`s one JSON-RPC
//! request to `/` with an OIDC access token. The answer is JSON, or for `turn/start` a
//! `text/event-stream` of the turn's notifications ending with its response. The method may
//! also be named in the `Glave-Method` header, for proxies and logs; when it is, it must match.
//! Every request needs a valid sign-in: without one the answer is HTTP 401 with error `-32013`.

mod body;

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
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use self::body::Answer;
use crate::protocol::{ApiError, Request, code};
use crate::rpc::{Id, error, parse, respond};
use crate::server::Server;
use crate::signin::{SignIn, SignInError};

/// The largest request body accepted, in bytes.
const MAX_BODY: usize = 16 * 1024 * 1024;

/// Connections served at once.
const MAX_CONNECTIONS: usize = 1_024;

/// How long a client may take to send a request's headers.
const HEADER_TIMEOUT: Duration = Duration::from_secs(30);

/// The header that names the method.
const METHOD_HEADER: &str = "glave-method";

/// Serves `server` over HTTP on `listener`, each request signed in through `signin`, until
/// `stop` completes. Then it cancels running turns and closes every connection.
pub async fn serve<P, L, B, S>(
    server: Server<P, L, B>,
    signin: SignIn,
    listener: TcpListener,
    stop: S,
) -> std::io::Result<()>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
    S: Future<Output = ()>,
{
    let signin = Arc::new(signin);
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
        let Ok((stream, _)) = accepted else {
            continue;
        };
        let (server, signin) = (server.clone(), Arc::clone(&signin));
        connections.spawn(async move {
            let service = service_fn(move |request| {
                let (server, signin) = (server.clone(), Arc::clone(&signin));
                async move { Ok::<_, Infallible>(answer(&server, &signin, request).await) }
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
    let caller = match bearer(request.headers()) {
        Some(token) => signin.verify(token).await,
        None => Err(SignInError::Missing),
    };
    let caller = match caller {
        Ok(caller) => caller,
        Err(refused) => return unauthorized(&refused),
    };
    let named = request
        .headers()
        .get(METHOD_HEADER)
        .map(|value| value.to_str().unwrap_or("").to_owned());
    let line = match read(request.into_body()).await {
        Ok(line) => line,
        Err(failed) => return json(StatusCode::OK, respond(None, Err(&failed))),
    };
    let (id, request) = match parse(&line) {
        Ok(parsed) => parsed,
        Err((id, failed)) => return json(StatusCode::OK, respond(id.as_ref(), Err(&failed))),
    };
    if let Some(named) = named
        && named != request.method()
    {
        let failed = ApiError::new(
            error::INVALID_REQUEST,
            format!(
                "the Glave-Method header says {named:?}, the request {:?}",
                request.method()
            ),
        );
        return json(StatusCode::OK, respond(id.as_ref(), Err(&failed)));
    }
    dispatch(server, &caller, id, request).await
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
    let answered = match server.start_streaming(caller, request).await {
        // A notification is never answered; the turn goes on without anyone following.
        Ok(Ok(_)) if id.is_none() => return accepted(),
        Ok(Ok(stream)) => return events(Answer::turn(stream, id)),
        Ok(Err(failed)) => Err(failed),
        Err(other) => server.request_as(caller, *other).await,
    };
    match id {
        Some(id) => json(StatusCode::OK, respond(Some(&id), answered.as_ref())),
        None => accepted(),
    }
}

/// The token in `Authorization: Bearer <token>`, if there is one.
fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

/// The request body as text, up to `MAX_BODY` bytes.
async fn read(body: Incoming) -> Result<String, ApiError> {
    let failed = |message: String| ApiError::new(error::PARSE, message);
    let mut body = pin!(body);
    let mut bytes = Vec::new();
    while let Some(frame) = poll_fn(|cx| Pin::as_mut(&mut body).poll_frame(cx)).await {
        let frame = frame.map_err(|e| failed(format!("could not read the request: {e}")))?;
        if let Ok(data) = frame.into_data() {
            if bytes.len().saturating_add(data.len()) > MAX_BODY {
                return Err(failed(format!("a request is longer than {MAX_BODY} bytes")));
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
