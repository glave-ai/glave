//! Why a server did not start, in words a person can act on rather than rmcp's types.

use std::error::Error;
use std::io::ErrorKind;

use rmcp::service::{ClientInitializeError, ServiceError};
use rmcp::transport::streamable_http_client::{
    AuthRequiredError, InsufficientScopeError, StreamableHttpError,
};

/// What a server that wants the person signed in is said to ask, in words they can act on.
pub(crate) const SIGN_IN: &str = "asks you to sign in to it: connect it, then try again";

/// `text` with each address's user, password, query and fragment left out, where a key in a
/// server's address would be: why a server failed is kept in the thread and shown to clients.
pub(crate) fn without_url_secrets(text: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let (before, after) = rest.split_at(at + 3);
        let end = after
            .find(|c: char| c.is_whitespace() || matches!(c, ')' | '"' | '\'' | '>'))
            .unwrap_or(after.len());
        let (address, others) = after.split_at(end);
        // The sentence's own colon or comma, after an address it follows.
        let stop = address.trim_end_matches([':', ',', '.']);
        let trailing = address.get(stop.len()..).unwrap_or_default();
        let bare = stop.split(['?', '#']).next().unwrap_or_default();
        let (authority, path) = bare.split_at(bare.find('/').unwrap_or(bare.len()));
        let host = authority.rsplit('@').next().unwrap_or_default();
        kept.push_str(before);
        kept.push_str(host);
        kept.push_str(path);
        kept.push_str(trailing);
        rest = others;
    }
    kept.push_str(rest);
    kept
}

/// Why a call to `server` failed with `err`.
pub(crate) fn failed(server: &str, err: &ServiceError) -> String {
    without_url_secrets(&failed_as_said(server, err))
}

fn failed_as_said(server: &str, err: &ServiceError) -> String {
    if let ServiceError::TransportSend(sent) = err {
        if asks_sign_in(&*sent.error) {
            return format!("{server} {SIGN_IN}.");
        }
        if let Some(scope) = asks_more(&*sent.error) {
            return format!("{server} {}.", sign_in_for(&scope));
        }
    }
    format!("{server}: {err}")
}

/// What a server that wants the person to sign in again, for more than they granted, is said
/// to ask, `scope` as it named it.
fn sign_in_for(scope: &str) -> String {
    format!("asks you to sign in to it again for more access ({scope}): connect it, then try again")
}

/// Whether `err`, or what caused it, is a server answering that the person must sign in
/// (HTTP 401).
fn asks_sign_in(err: &(dyn Error + 'static)) -> bool {
    causes(err).any(<dyn Error>::is::<AuthRequiredError>)
}

/// What a server asks for when `err`, or what caused it, is it answering that the person's
/// sign-in grants too little (HTTP 403 `insufficient_scope`, naming the scope).
fn asks_more(err: &(dyn Error + 'static)) -> Option<String> {
    causes(err)
        .find_map(|err| crate::signed::challenged(err.downcast_ref::<InsufficientScopeError>()?))
}

/// `err`, then each thing that caused it.
pub(crate) fn causes<'a>(
    err: &'a (dyn Error + 'static),
) -> impl Iterator<Item = &'a (dyn Error + 'static)> {
    std::iter::successors(Some(err), |&err| err.source())
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
    if let Some(scope) = asks_more(err) {
        return format!("it {}", sign_in_for(&scope));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_an_address_may_hide_is_left_out_of_why_a_server_failed() {
        for (said, kept) in [
            (
                "could not reach https://mcp.example.com/mcp?key=private: refused",
                "could not reach https://mcp.example.com/mcp: refused",
            ),
            (
                "error sending request for url (http://me:private@host:8/a#private)",
                "error sending request for url (http://host:8/a)",
            ),
            (
                "it did not answer within 30 s",
                "it did not answer within 30 s",
            ),
            (
                "at http://host/a, then http://b/?x=private.",
                "at http://host/a, then http://b/.",
            ),
        ] {
            assert_eq!(without_url_secrets(said), kept);
        }
    }
}
