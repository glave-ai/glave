//! How a model provider proves who it is: a fixed key in the header the provider wants, or
//! credentials that change, such as a bearer token refreshed before it expires or a signature
//! over each request. Secrets here never show in `Debug`, in `Display` or in errors.

mod cache;

use std::fmt;
use std::sync::Arc;

use thiserror::Error;

use crate::model::ProviderError;
use crate::tools::BoxFuture;

pub use cache::{Fresh, Refreshing};
pub use glave_event::Secret;

/// The header a fixed key goes in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyHeader {
    /// `Authorization: Bearer <key>`: OpenAI's, and most compatible servers'.
    Bearer,
    /// `api-key: <key>`: Azure OpenAI's.
    ApiKey,
    /// `x-api-key: <key>`: Anthropic's.
    XApiKey,
}

impl KeyHeader {
    /// The header's name, in lower case.
    pub fn name(self) -> &'static str {
        match self {
            Self::Bearer => "authorization",
            Self::ApiKey => "api-key",
            Self::XApiKey => "x-api-key",
        }
    }

    /// The header's value for `key`.
    fn value(self, key: &Secret) -> Secret {
        match self {
            Self::Bearer => Secret::new(format!("Bearer {}", key.expose())),
            Self::ApiKey | Self::XApiKey => key.clone(),
        }
    }
}

/// A header that proves who is calling. Its value is secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub name: String,
    pub value: Secret,
}

/// The request about to go out, for a [`RequestSigner`].
#[derive(Debug, Clone, Copy)]
pub struct Outgoing<'a> {
    pub method: &'a str,
    pub url: &'a str,
    /// The headers already set, besides the ones credentials add.
    pub headers: &'a [(&'a str, &'a str)],
    pub body: &'a [u8],
}

/// Credentials that change, signing each request: each request asks the signer for the
/// headers that prove who is calling.
pub trait RequestSigner: Send + Sync {
    /// The headers that authenticate `request`: a fresh bearer token, or a signature over it.
    fn authorize<'a>(
        &'a self,
        request: &'a Outgoing<'a>,
    ) -> BoxFuture<'a, Result<Vec<Header>, AuthError>>;

    /// Where the credentials come from, to say what to fix, such as `Microsoft Entra
    /// workload identity`. Never a secret.
    fn describe(&self) -> String;
}

/// How a provider proves who it is on each request.
#[derive(Clone, Default)]
pub enum Auth {
    /// Nothing: a server that wants no key, such as one on this computer.
    #[default]
    None,
    /// A fixed key, in the header the provider wants.
    Key { header: KeyHeader, key: Secret },
    /// Credentials that change: a signer asked on each request.
    Changing(Arc<dyn RequestSigner>),
}

impl Auth {
    /// `key`, when there is one, as a bearer token: the consumer's one key.
    pub fn bearer(key: Option<String>) -> Self {
        key.map_or(Self::None, |key| Self::Key {
            header: KeyHeader::Bearer,
            key: Secret::new(key),
        })
    }

    /// `key` in `header`.
    pub fn key(header: KeyHeader, key: impl Into<String>) -> Self {
        Self::Key {
            header,
            key: Secret::new(key),
        }
    }

    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// The fixed key, when there is one: for the list of values never to show.
    pub fn fixed_key(&self) -> Option<&Secret> {
        match self {
            Self::Key { key, .. } => Some(key),
            Self::None | Self::Changing(_) => None,
        }
    }

    /// The same credentials with a fixed key moved to `header`; others are kept as they are.
    #[must_use]
    pub fn in_header(self, header: KeyHeader) -> Self {
        match self {
            Self::Key { key, .. } => Self::Key { header, key },
            other => other,
        }
    }

    /// The headers that authenticate `request`.
    pub async fn headers(&self, request: &Outgoing<'_>) -> Result<Vec<Header>, AuthError> {
        match self {
            Self::None => Ok(Vec::new()),
            Self::Key { header, key } => Ok(vec![Header {
                name: header.name().to_owned(),
                value: header.value(key),
            }]),
            Self::Changing(signer) => signer.authorize(request).await,
        }
    }

    /// Where the credentials come from, to say what to fix.
    pub fn describe(&self) -> String {
        match self {
            Self::None => "no key".to_owned(),
            Self::Key { header, .. } => format!("the key in {}", header.name()),
            Self::Changing(signer) => signer.describe(),
        }
    }
}

impl fmt::Debug for Auth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => f.write_str("Auth::None"),
            Self::Key { header, key } => f
                .debug_struct("Auth::Key")
                .field("header", header)
                .field("key", key)
                .finish(),
            Self::Changing(signer) => write!(f, "Auth::Changing({})", signer.describe()),
        }
    }
}

/// Credentials could not be had. The message names where they come from and what went wrong,
/// never a secret.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct AuthError {
    message: String,
    /// Whether asking again may work: the source could not be reached, or failed on its side.
    transient: bool,
}

impl AuthError {
    /// Credentials that are wrong or missing: asking again fails the same way.
    pub fn refused(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            transient: false,
        }
    }

    /// A source that could not be reached, or failed on its side.
    pub fn unreachable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            transient: true,
        }
    }

    /// What a source that answered HTTP `status` meant: `what` it refused, then `detail` from
    /// its answer, cut short. Too many requests, or a failure on the source's side, may pass
    /// if asked again; anything else is refused.
    pub fn answered(status: u16, what: &str, detail: &str) -> Self {
        let detail: String = detail.chars().take(DETAIL_CHARS).collect();
        let message = format!("{what}: {detail}");
        if status == 429 || status >= 500 {
            Self::unreachable(message)
        } else {
            Self::refused(message)
        }
    }

    pub fn is_transient(&self) -> bool {
        self.transient
    }
}

/// The most of a source's answer an error repeats, in characters.
const DETAIL_CHARS: usize = 300;

/// Whether credentials may be sent to `url`: over https, or over plain http only to this
/// computer or one of `plain_hosts`, and never to a URL that names a user or a password.
pub fn safe_to_send(url: &str, plain_hosts: &[&str]) -> bool {
    let Some((scheme, rest)) = url.trim().split_once("://") else {
        return false;
    };
    if signs_in(url) {
        return false;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = match authority.rsplit_once(':') {
        Some((host, port)) if port.bytes().all(|b| b.is_ascii_digit()) => host,
        _ => authority,
    };
    match scheme {
        "https" => !host.is_empty(),
        "http" => crate::host::is_this_computer(host) || plain_hosts.contains(&host),
        _ => false,
    }
}

/// Whether `url` carries a user name or password before its host, where a mistake could put
/// a secret. An `@` later, in a path such as a service account's email, is fine.
pub fn signs_in(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split(['/', '?', '#'])
        .next()
        .is_some_and(|authority| authority.contains('@'))
}

/// The shortest secret redacted: shorter strings are too common to be secrets, and a
/// scheme's name such as `Bearer` stays readable.
const SHORTEST_SECRET: usize = 8;

/// `text` with each of `secrets` at least 8 characters long replaced by `[redacted]`.
pub fn redact<'a>(text: &str, secrets: impl IntoIterator<Item = &'a str>) -> String {
    let mut text = text.to_owned();
    for secret in secrets {
        if secret.len() >= SHORTEST_SECRET && text.contains(secret) {
            text = text.replace(secret, "[redacted]");
        }
    }
    text
}

/// `text` with the credentials of `sent` redacted: each header's whole value, and each word
/// of it, such as the token after `Bearer`. For a provider's error, since some repeat the key
/// they were sent.
fn redact_credentials(text: &str, sent: &[Header]) -> String {
    let values = sent.iter().map(|header| header.value.expose());
    redact(
        text,
        values.flat_map(|value| std::iter::once(value).chain(value.split_whitespace())),
    )
}

/// The most of a provider's error body read: far more than any message, never all of a page
/// a proxy or a failing server may send.
pub const ERROR_BODY_BYTES: usize = 64 << 10;

/// How much of an error's message is kept, in characters.
const ERROR_CHARS: usize = 500;

impl ProviderError {
    /// A provider's answer with the error status `status`, its body `body` (at most
    /// [`ERROR_BODY_BYTES`] of it read), to a request that `sent` credentials: the message in
    /// the fields OpenAI, OpenRouter, Ollama, Anthropic and Google send it in (Vertex AI's
    /// sometimes in a one-element array), or else the body itself, on one line, cut short,
    /// and with the credentials redacted, since some providers repeat the key they were sent.
    pub fn from_response(status: u16, body: &[u8], sent: &[Header]) -> Self {
        let body = String::from_utf8_lossy(body.get(..ERROR_BODY_BYTES).unwrap_or(body));
        let body = redact_credentials(&body, sent);
        let parsed: Option<serde_json::Value> = serde_json::from_str(&body).ok();
        let message = parsed.as_ref().and_then(|value| {
            [
                value.pointer("/error/message"),
                value.pointer("/0/error/message"),
                value.get("error"),
                value.get("message"),
                value.get("detail"),
            ]
            .into_iter()
            .flatten()
            .find_map(serde_json::Value::as_str)
        });
        Self::Http {
            status,
            body: error_line(message.unwrap_or(&body)),
        }
    }
}

/// A provider's error message on one readable line, cut short.
pub fn error_line(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    line.chars().take(ERROR_CHARS).collect()
}

impl From<AuthError> for ProviderError {
    fn from(error: AuthError) -> Self {
        if error.transient {
            Self::Transport(error.message)
        } else {
            Self::Credentials(error.message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_on;
    use crate::testing::FAKE_SECRET;

    const REQUEST: Outgoing<'static> = Outgoing {
        method: "POST",
        url: "https://models.example.com/v1/chat/completions",
        headers: &[],
        body: b"{}",
    };

    fn headers(auth: &Auth) -> Vec<(String, String)> {
        block_on!(auth.headers(&REQUEST))
            .unwrap()
            .into_iter()
            .map(|header| (header.name, header.value.expose().to_owned()))
            .collect()
    }

    #[test]
    fn a_fixed_key_goes_in_the_header_the_provider_wants() {
        let pair = |name: &str, value: &str| vec![(name.to_owned(), value.to_owned())];
        assert_eq!(
            headers(&Auth::bearer(Some(FAKE_SECRET.into()))),
            pair("authorization", &format!("Bearer {FAKE_SECRET}"))
        );
        assert_eq!(
            headers(&Auth::key(KeyHeader::ApiKey, "az-1")),
            pair("api-key", "az-1")
        );
        assert_eq!(
            headers(&Auth::bearer(Some(FAKE_SECRET.into())).in_header(KeyHeader::XApiKey)),
            pair("x-api-key", FAKE_SECRET)
        );
        assert_eq!(headers(&Auth::bearer(None)), Vec::new());
    }

    struct Signer;

    impl RequestSigner for Signer {
        fn authorize<'a>(
            &'a self,
            request: &'a Outgoing<'a>,
        ) -> BoxFuture<'a, Result<Vec<Header>, AuthError>> {
            Box::pin(async move {
                Ok(vec![Header {
                    name: "x-signed".into(),
                    value: Secret::new(format!("{} {}", request.method, request.body.len())),
                }])
            })
        }

        fn describe(&self) -> String {
            "a test signer".into()
        }
    }

    #[test]
    fn changing_credentials_see_the_request() {
        let auth = Auth::Changing(Arc::new(Signer));
        assert_eq!(
            headers(&auth),
            vec![("x-signed".to_owned(), "POST 2".to_owned())]
        );
        assert_eq!(auth.describe(), "a test signer");
        assert!(auth.fixed_key().is_none());
    }

    #[test]
    fn secrets_never_show_in_debug() {
        let shown = format!(
            "{:?} {:?} {:?}",
            Auth::key(KeyHeader::ApiKey, "hunter2"),
            Secret::new("hunter2"),
            block_on!(Auth::bearer(Some("hunter2".into())).headers(&REQUEST)).unwrap(),
        );
        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(shown.contains("[redacted]"), "{shown}");
        assert_eq!(
            format!("{:?}", Auth::Changing(Arc::new(Signer))),
            "Auth::Changing(a test signer)"
        );
    }

    #[test]
    fn credentials_a_server_repeats_are_redacted() {
        let sent = block_on!(Auth::bearer(Some(FAKE_SECRET.into())).headers(&REQUEST)).unwrap();
        let said = format!("Incorrect API key provided: {FAKE_SECRET}. Bearer tokens only.");
        assert_eq!(
            redact_credentials(&said, &sent),
            "Incorrect API key provided: [redacted]. Bearer tokens only."
        );
        let signed = vec![Header {
            name: "authorization".into(),
            value: Secret::new("AWS4-HMAC-SHA256 Signature=0123456789abcdef"),
        }];
        assert_eq!(
            redact_credentials("bad Signature=0123456789abcdef", &signed),
            "bad [redacted]"
        );
        assert_eq!(redact("short key", ["short"]), "short key");
    }

    #[test]
    fn an_error_response_is_its_message_on_one_line_with_no_key_in_it() {
        let message = |body: &str| match ProviderError::from_response(400, body.as_bytes(), &[]) {
            ProviderError::Http { status: 400, body } => body,
            other => panic!("not HTTP 400: {other:?}"),
        };
        let said = [
            // OpenAI's, OpenRouter's and Anthropic's.
            r#"{"error":{"message":"nosuch/x is not a valid model ID","code":400},"user_id":"u_1"}"#,
            // Vertex AI's, Gemini's as much as Claude's.
            r#"[{"error":{"code":404,"message":"nosuch/x is not a valid model ID","status":"NOT_FOUND"}}]"#,
            // Ollama's, then others'.
            r#"{"error":"nosuch/x is not a valid model ID"}"#,
            r#"{"message":"nosuch/x is not a valid model ID"}"#,
            r#"{"detail":"nosuch/x is not a valid model ID"}"#,
        ];
        for body in said {
            assert_eq!(message(body), "nosuch/x is not a valid model ID", "{body}");
        }
        assert_eq!(
            message("<html>\n  <body>Bad Gateway</body>\n</html>"),
            "<html> <body>Bad Gateway</body> </html>"
        );
        assert_eq!(message(&"x".repeat(900)).len(), ERROR_CHARS);
        let past = format!(
            r#"{{"padding":"{}","error":{{"message":"never read"}}}}"#,
            " ".repeat(ERROR_BODY_BYTES)
        );
        assert_eq!(
            message(&past),
            r#"{"padding":""#,
            "read no further than the most"
        );
        let sent = block_on!(Auth::bearer(Some(FAKE_SECRET.into())).headers(&REQUEST)).unwrap();
        let repeated = format!(r#"{{"error":{{"message":"Incorrect API key: {FAKE_SECRET}"}}}}"#);
        assert_eq!(
            ProviderError::from_response(401, repeated.as_bytes(), &sent),
            ProviderError::Http {
                status: 401,
                body: "Incorrect API key: [redacted]".into()
            }
        );
    }

    #[test]
    fn a_failure_to_get_credentials_is_refused_or_transient() {
        let refused: ProviderError = AuthError::refused("no token file").into();
        assert_eq!(refused.failure(), crate::Failure::Refused);
        let transient: ProviderError = AuthError::unreachable("STS is down").into();
        assert!(transient.is_transient());
    }

    #[test]
    fn a_refusal_says_what_and_why_and_whether_to_ask_again() {
        let refused = AuthError::answered(400, "STS refused", "InvalidIdentityToken");
        assert_eq!(refused.to_string(), "STS refused: InvalidIdentityToken");
        assert!(!refused.is_transient());
        assert!(AuthError::answered(429, "x", "").is_transient());
        assert!(AuthError::answered(503, "x", "").is_transient());
        let long = AuthError::answered(400, "x", &"y".repeat(900));
        assert_eq!(long.to_string().len(), "x: ".len() + DETAIL_CHARS);
    }

    #[test]
    fn credentials_go_over_https_or_to_this_computer_never_in_the_url() {
        for url in [
            "https://sts.amazonaws.com",
            "http://127.0.0.1:4566",
            "http://localhost",
            "http://[::1]:8080/token",
            "https://iam.googleapis.com/v1/projects/-/serviceAccounts/a@b.iam.gserviceaccount.com",
        ] {
            assert!(safe_to_send(url, &[]), "{url}");
        }
        for url in [
            "http://sts.example.com",
            "http://localhost.evil.com",
            "http://127.0.0.1.example.com",
            "http://localhost@example.com",
            "not a url",
            "https://user:pass@sts.example.com",
            "ftp://localhost",
            "localhost:80",
        ] {
            assert!(!safe_to_send(url, &[]), "{url}");
        }
        assert!(safe_to_send(
            "http://169.254.170.2/v2/creds",
            &["169.254.170.2"]
        ));
    }
}
