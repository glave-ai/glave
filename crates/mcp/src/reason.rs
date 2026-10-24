//! Why a server did not start, in words a person can act on rather than rmcp's types.

use std::error::Error;
use std::io::ErrorKind;

use rmcp::service::{ClientInitializeError, ServiceError};
use rmcp::transport::streamable_http_client::{AuthRequiredError, StreamableHttpError};

/// What a server that wants the person signed in is said to ask, in words they can act on.
pub(crate) const SIGN_IN: &str = "asks you to sign in to it: connect it, then try again";

/// Why a call to `server` failed with `err`.
pub(crate) fn failed(server: &str, err: &ServiceError) -> String {
    match err {
        ServiceError::TransportSend(sent) if asks_sign_in(&*sent.error) => {
            format!("{server} {SIGN_IN}.")
        }
        other => format!("{server}: {other}"),
    }
}

/// Whether `err`, or what caused it, is a server answering that the person must sign in
/// (HTTP 401).
fn asks_sign_in(err: &(dyn Error + 'static)) -> bool {
    let mut current = Some(err);
    while let Some(err) = current {
        if err.is::<AuthRequiredError>() {
            return true;
        }
        current = err.source();
    }
    false
}

pub(crate) fn reason(err: &ClientInitializeError) -> String {
    match err {
        ClientInitializeError::ConnectionClosed(_) => exited(),
        ClientInitializeError::TransportError { error, .. } => transport(&*error.error),
        ClientInitializeError::LegacyFallbackFailed { fallback, .. } => reason(fallback),
        ClientInitializeError::NoCompatibleProtocolVersion {
            server_supported, ..
        } => {
            let versions: Vec<String> = server_supported.iter().map(ToString::to_string).collect();
            format!(
                "it speaks only MCP {}, which glave does not",
                versions.join(", ")
            )
        }
        other => other.to_string(),
    }
}

fn exited() -> String {
    "it exited or closed the connection before answering".to_owned()
}

fn transport(err: &(dyn Error + Send + Sync + 'static)) -> String {
    if asks_sign_in(err) {
        return format!("it {SIGN_IN}");
    }
    if let Some(io) = err.downcast_ref::<std::io::Error>()
        && matches!(io.kind(), ErrorKind::BrokenPipe | ErrorKind::UnexpectedEof)
    {
        return exited();
    }
    if let Some(StreamableHttpError::Client(client)) =
        err.downcast_ref::<StreamableHttpError<reqwest::Error>>()
    {
        let cause = innermost(client);
        return match client.url() {
            Some(url) => format!("could not reach {url}: {cause}"),
            None => format!("{client}: {cause}"),
        };
    }
    err.to_string()
}

/// The deepest cause of `err`, which is usually the one that says what to fix.
fn innermost(err: &(dyn Error + 'static)) -> String {
    let mut cause = err;
    while let Some(source) = cause.source() {
        cause = source;
    }
    cause.to_string()
}
