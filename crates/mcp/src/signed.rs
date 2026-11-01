//! How a remote server is reached: rmcp's own HTTP client, with what proves who is calling
//! asked for on every request, so a token renewed meanwhile goes with the next one, and told
//! when a server asks for more than it grants. The headers are marked sensitive, which keeps
//! them out of what HTTP's own tools print.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use glave_ports::auth::{Auth, Outgoing};
use http::{HeaderName, HeaderValue};
use rmcp::model::ClientJsonRpcMessage;
use rmcp::transport::auth::AuthError;
use rmcp::transport::common::client_side_sse::BoxedSseResponse;
use rmcp::transport::streamable_http_client::{
    InsufficientScopeError, StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse,
};

use crate::reachable::Reachable;

type Failed = StreamableHttpError<reqwest::Error>;

/// How long reaching a remote server may take: moments; one that has not answered by then is
/// down or out of reach.
const CONNECTS_WITHIN: Duration = Duration::from_secs(10);
/// How long a server has to answer the end of a session, as glave closes the connection.
const ENDS_WITHIN: Duration = Duration::from_secs(3);

/// rmcp's client, with `auth`'s headers on every request.
#[derive(Clone)]
pub(crate) struct Signed {
    http: reqwest::Client,
    auth: Auth,
}

impl Signed {
    /// A client that follows no redirect, as rmcp's own, so what proves who is calling never
    /// reaches another host; looks names up under `reachable`; keeps no idle connection, which rmcp found stalls; and reaches
    /// the server within [`CONNECTS_WITHIN`]. Each call has its own limit (`connection.rs`):
    /// a server's stream of notifications has none, since it is quiet until something changes.
    pub(crate) fn new(auth: Auth, reachable: Reachable) -> Result<Self, reqwest::Error> {
        let http = reachable
            .client()
            .connect_timeout(CONNECTS_WITHIN)
            .pool_max_idle_per_host(0)
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self { http, auth })
    }

    /// `headers` with what proves who is calling `method` at `uri` added. A header the signer
    /// gives that cannot be sent fails the request: it is never sent without it.
    async fn signed(
        &self,
        method: &str,
        uri: &str,
        mut headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<HashMap<HeaderName, HeaderValue>, Failed> {
        let outgoing = Outgoing {
            method,
            url: uri,
            headers: &[],
            body: &[],
        };
        let refused = |message: String| Failed::Auth(AuthError::CredentialStoreError(message));
        let proofs = self
            .auth
            .headers(&outgoing)
            .await
            .map_err(|error| refused(error.to_string()))?;
        for proof in proofs {
            let name = HeaderName::try_from(proof.name.as_str())
                .map_err(|_| refused(format!("{:?} is not a header name", proof.name)))?;
            let mut value = HeaderValue::try_from(proof.value.expose())
                .map_err(|_| refused(format!("the value of header {name} is not valid")))?;
            value.set_sensitive(true);
            headers.insert(name, value);
        }
        Ok(headers)
    }

    /// `answered`, with the signer told what a server that answered `403 insufficient_scope`
    /// asked for, so the person can sign in again for it.
    fn heard<T>(&self, answered: Result<T, Failed>) -> Result<T, Failed> {
        if let Err(Failed::InsufficientScope(asked)) = &answered
            && let Some(scope) = challenged(asked)
        {
            self.auth.insufficient(&scope);
        }
        answered
    }
}

/// The most scopes, and the longest scope, a challenge may name to be heard.
const SCOPES: usize = 16;
const SCOPE_LENGTH: usize = 128;

/// The scopes a server's `403` asks for, when it is `insufficient_scope` naming scopes as
/// OAuth writes them (RFC 6749, section 3.3), at most [`SCOPES`] of [`SCOPE_LENGTH`]: none
/// for any other refusal, so what a server sends reaches the person's sign-in and the model
/// only as plain scope names. rmcp reads every `403` with a challenge as this one.
pub(crate) fn challenged(asked: &InsufficientScopeError) -> Option<String> {
    let header = &asked.www_authenticate_header;
    if !header.contains("error=\"insufficient_scope\"")
        && !header.contains("error=insufficient_scope")
    {
        return None;
    }
    let scopes: Vec<&str> = asked.get_required_scope()?.split_whitespace().collect();
    let plain = |scope: &&str| {
        scope.len() <= SCOPE_LENGTH
            && scope
                .bytes()
                .all(|b| b == 0x21 || (0x23..=0x5b).contains(&b) || (0x5d..=0x7e).contains(&b))
    };
    let fits = !scopes.is_empty() && scopes.len() <= SCOPES && scopes.iter().all(plain);
    fits.then(|| scopes.join(" "))
}

impl StreamableHttpClient for Signed {
    type Error = reqwest::Error;

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, Failed> {
        let headers = self.signed("POST", &uri, custom_headers).await?;
        let answered = self
            .http
            .post_message(uri, message, session_id, auth_header, headers)
            .await;
        self.heard(answered)
    }

    async fn post_message_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
        max_sse_event_size: usize,
    ) -> Result<StreamableHttpPostResponse, Failed> {
        let headers = self.signed("POST", &uri, custom_headers).await?;
        let answered = self
            .http
            .post_message_with_max_sse_event_size(
                uri,
                message,
                session_id,
                auth_header,
                headers,
                max_sse_event_size,
            )
            .await;
        self.heard(answered)
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), Failed> {
        let headers = self.signed("DELETE", &uri, custom_headers).await?;
        // Ending the session is a courtesy: one the server does not answer in time ends with
        // the connection all the same, so closing never waits on it.
        let ended = self
            .http
            .delete_session(uri, session_id, auth_header, headers);
        tokio::time::timeout(ENDS_WITHIN, ended)
            .await
            .unwrap_or(Ok(()))
    }

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxedSseResponse, Failed> {
        let headers = self.signed("GET", &uri, custom_headers).await?;
        let answered = self
            .http
            .get_stream(uri, session_id, last_event_id, auth_header, headers)
            .await;
        self.heard(answered)
    }

    async fn get_stream_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
        max_sse_event_size: usize,
    ) -> Result<BoxedSseResponse, Failed> {
        let headers = self.signed("GET", &uri, custom_headers).await?;
        let answered = self
            .http
            .get_stream_with_max_sse_event_size(
                uri,
                session_id,
                last_event_id,
                auth_header,
                headers,
                max_sse_event_size,
            )
            .await;
        self.heard(answered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_insufficient_scope_naming_plain_scopes_is_heard() {
        let asked = |header: &str, scope: &str| {
            let asked = InsufficientScopeError::new(header.to_owned(), Some(scope.to_owned()));
            challenged(&asked)
        };
        let challenge = r#"Bearer error="insufficient_scope", scope="files:write a""#;
        assert_eq!(
            asked(challenge, "files:write  a").as_deref(),
            Some("files:write a")
        );
        let other = r#"Bearer error="invalid_token", scope="files:write""#;
        assert_eq!(asked(other, "files:write"), None, "not asking for more");
        for odd in ["say \"hi\"", "back\\slash", "", &"x".repeat(129)] {
            assert_eq!(asked(challenge, odd), None, "{odd:?}");
        }
        let many = vec!["s"; 17].join(" ");
        assert_eq!(asked(challenge, &many), None);
    }

    #[test]
    fn a_server_that_never_answers_the_end_of_a_session_is_left_in_seconds() {
        // Listening, never answering: the system takes the connection and nothing reads it.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let uri: Arc<str> = format!("http://{}/mcp", listener.local_addr().unwrap()).into();
        let signed = Signed::new(Auth::None, Reachable::default()).unwrap();
        let (ended, waited) = glave_ports::block_on!(async {
            // The clock jumps to the limit as soon as nothing else can run.
            tokio::time::pause();
            let started = tokio::time::Instant::now();
            let ended = signed
                .delete_session(uri, "a-session".into(), None, HashMap::new())
                .await;
            (ended, started.elapsed())
        });
        assert!(ended.is_ok(), "{ended:?}");
        assert!(waited < Duration::from_secs(4), "{waited:?}");
    }
}
