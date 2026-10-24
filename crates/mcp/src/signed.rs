//! How a remote server is reached: rmcp's own HTTP client, with what proves who is calling
//! asked for on every request, so a token renewed meanwhile goes with the next one. The
//! headers are marked sensitive, which keeps them out of what HTTP's own tools print.

use std::collections::HashMap;
use std::sync::Arc;

use glave_ports::auth::{Auth, Outgoing};
use http::{HeaderName, HeaderValue};
use rmcp::model::ClientJsonRpcMessage;
use rmcp::transport::auth::AuthError;
use rmcp::transport::common::client_side_sse::BoxedSseResponse;
use rmcp::transport::streamable_http_client::{
    StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse,
};

type Failed = StreamableHttpError<reqwest::Error>;

/// rmcp's client, with `auth`'s headers on every request.
#[derive(Clone)]
pub(crate) struct Signed {
    http: reqwest::Client,
    auth: Auth,
}

impl Signed {
    /// A client that follows no redirect, as rmcp's own, so what proves who is calling never
    /// reaches another host; and keeps no idle connection, which rmcp found stalls.
    pub(crate) fn new(auth: Auth) -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder()
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
        self.http
            .post_message(uri, message, session_id, auth_header, headers)
            .await
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
        self.http
            .post_message_with_max_sse_event_size(
                uri,
                message,
                session_id,
                auth_header,
                headers,
                max_sse_event_size,
            )
            .await
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), Failed> {
        let headers = self.signed("DELETE", &uri, custom_headers).await?;
        self.http
            .delete_session(uri, session_id, auth_header, headers)
            .await
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
        self.http
            .get_stream(uri, session_id, last_event_id, auth_header, headers)
            .await
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
        self.http
            .get_stream_with_max_sse_event_size(
                uri,
                session_id,
                last_event_id,
                auth_header,
                headers,
                max_sse_event_size,
            )
            .await
    }
}
