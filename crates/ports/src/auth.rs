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

/// A key or token. Its `Debug` never shows it, and it has no `Display`.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The secret itself, to send. Never to log or show.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([redacted])")
    }
}

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
            Self::Bearer => Secret(format!("Bearer {}", key.expose())),
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

/// The request about to go out, for credentials that sign it.
#[derive(Debug, Clone, Copy)]
pub struct Outgoing<'a> {
    pub method: &'a str,
    pub url: &'a str,
    /// The headers already set, besides the ones credentials add.
    pub headers: &'a [(&'a str, &'a str)],
    pub body: &'a [u8],
}

/// Credentials that change: each request asks them for the headers that prove who is calling.
pub trait Credentials: Send + Sync {
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
    /// Credentials that change.
    Changing(Arc<dyn Credentials>),
}

impl Auth {
    /// `key`, when there is one, as a bearer token: the consumer's one key.
    pub fn bearer(key: Option<String>) -> Self {
        key.map_or(Self::None, |key| Self::Key {
            header: KeyHeader::Bearer,
            key: Secret(key),
        })
    }

    /// `key` in `header`.
    pub fn key(header: KeyHeader, key: impl Into<String>) -> Self {
        Self::Key {
            header,
            key: Secret(key.into()),
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
            Self::Changing(credentials) => credentials.authorize(request).await,
        }
    }

    /// Where the credentials come from, to say what to fix.
    pub fn describe(&self) -> String {
        match self {
            Self::None => "no key".to_owned(),
            Self::Key { header, .. } => format!("the key in {}", header.name()),
            Self::Changing(credentials) => credentials.describe(),
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
            Self::Changing(credentials) => write!(f, "Auth::Changing({})", credentials.describe()),
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

    pub fn is_transient(&self) -> bool {
        self.transient
    }
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
            headers(&Auth::bearer(Some("sk-1".into()))),
            pair("authorization", "Bearer sk-1")
        );
        assert_eq!(
            headers(&Auth::key(KeyHeader::ApiKey, "az-1")),
            pair("api-key", "az-1")
        );
        assert_eq!(
            headers(&Auth::bearer(Some("sk-ant-1".into())).in_header(KeyHeader::XApiKey)),
            pair("x-api-key", "sk-ant-1")
        );
        assert_eq!(headers(&Auth::bearer(None)), Vec::new());
    }

    struct Signer;

    impl Credentials for Signer {
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
    fn a_failure_to_get_credentials_is_refused_or_transient() {
        let refused: ProviderError = AuthError::refused("no token file").into();
        assert_eq!(refused.failure(), crate::Failure::Refused);
        let transient: ProviderError = AuthError::unreachable("STS is down").into();
        assert!(transient.is_transient());
    }
}
