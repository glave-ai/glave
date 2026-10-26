//! Fetching one page: an HTTP GET of an http or https URL, redirects followed by hand so each
//! hop is checked again, the body read up to a size limit within a time limit, and turned
//! into text. Every address is checked: literal ones here, names as they resolve
//! (`resolve.rs`).

use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use glave_ports::settings::{HostRule, WebPolicy};
use reqwest::header::{ACCEPT, CONTENT_TYPE, LOCATION};
use reqwest::{Client, Url, redirect};
use thiserror::Error;

use crate::address::reachable;
use crate::cache::Cache;
use crate::resolve::{Guarded, Refused};
use crate::text::{kind, readable, title};

/// How much a fetch may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The most bytes of a body read; the rest is left unread.
    pub max_bytes: usize,
    /// The longest a whole fetch may take, redirects included.
    pub timeout: Duration,
    /// The most redirects followed.
    pub max_redirects: u8,
    /// The most characters of text the model is given.
    pub max_chars: usize,
}

impl Default for Limits {
    /// 5 MB, 30 seconds and 5 redirects, as OpenCode and Gemini CLI fetch; 50,000 characters,
    /// about 12,000 tokens.
    fn default() -> Self {
        Self {
            max_bytes: 5 * 1024 * 1024,
            timeout: Duration::from_secs(30),
            max_redirects: 5,
            max_chars: 50_000,
        }
    }
}

/// A page, as the model is given it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// Where the page was found, after redirects.
    pub url: String,
    /// The title it gives itself, when it gives one.
    pub title: Option<String>,
    pub text: String,
    /// The body was longer than the size limit, and only its start was read.
    pub cut: bool,
}

/// Why a page could not be fetched, in words the model can act on. Never holds page content.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FetchError {
    #[error("{0:?} is not a URL")]
    NotUrl(String),
    #[error("only http and https URLs can be fetched, not {0}:")]
    Scheme(String),
    #[error("URLs with a user name or password are not fetched")]
    Credentials,
    #[error("{host} is not fetched: the person's settings deny {rule}")]
    Denied { host: String, rule: String },
    #[error("{0} is not fetched: the person's settings allow only some hosts, and not this one")]
    NotAllowed(String),
    #[error("{0}")]
    Private(String),
    #[error("more than {0} redirects")]
    Redirects(u8),
    #[error("a redirect from {0} went nowhere readable")]
    BadRedirect(String),
    #[error("the server answered {status}")]
    Status { status: String },
    #[error("this is not text the model can read ({0})")]
    NotText(String),
    #[error("no answer within {0:?}")]
    Timeout(Duration),
    #[error("could not fetch it: {0}")]
    Failed(String),
}

/// Fetches pages under a policy and limits.
pub struct Fetcher {
    client: Client,
    policy: Arc<WebPolicy>,
    limits: Limits,
    cache: Cache<Page>,
}

impl Fetcher {
    /// A fetcher with its own client: no proxy (a proxy would resolve names where they are not
    /// checked), no automatic redirects, every name looked up through the policy.
    pub fn new(policy: WebPolicy, limits: Limits) -> Result<Self, FetchError> {
        let policy = Arc::new(policy);
        let client = Client::builder()
            .no_proxy()
            .redirect(redirect::Policy::none())
            .connect_timeout(limits.timeout)
            .dns_resolver(Arc::new(Guarded {
                policy: Arc::clone(&policy),
            }))
            .user_agent(concat!("glave/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| FetchError::Failed(e.to_string()))?;
        Ok(Self {
            client,
            policy,
            limits,
            cache: Cache::new(),
        })
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Fetches `url`, or gives the page fetched lately from it.
    pub async fn fetch(&self, url: &str) -> Result<Page, FetchError> {
        let url = Url::parse(url.trim()).map_err(|_| FetchError::NotUrl(url.to_owned()))?;
        let key = url.as_str().to_owned();
        if let Some(page) = self.cache.get(&key, Instant::now()) {
            return Ok(page);
        }
        let limit = self.limits.timeout;
        let page = tokio::time::timeout(limit, self.follow(url))
            .await
            .map_err(|_| FetchError::Timeout(limit))??;
        self.cache.put(key, page.clone(), Instant::now());
        Ok(page)
    }

    /// Follows redirects from `url`, checking each hop, and reads the page at the end.
    async fn follow(&self, mut url: Url) -> Result<Page, FetchError> {
        for _ in 0..=self.limits.max_redirects {
            self.check(&url)?;
            let response = self
                .client
                .get(url.clone())
                .header(
                    ACCEPT,
                    "text/html, text/markdown, text/plain, application/json;q=0.9, */*;q=0.5",
                )
                .send()
                .await
                .map_err(|e| failed(&e))?;
            let status = response.status();
            if status.is_redirection() {
                let next = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|location| url.join(location).ok())
                    .ok_or_else(|| FetchError::BadRedirect(url.to_string()))?;
                url = next;
                continue;
            }
            if !status.is_success() {
                return Err(FetchError::Status {
                    status: status.to_string(),
                });
            }
            return self.read(url, response).await;
        }
        Err(FetchError::Redirects(self.limits.max_redirects))
    }

    /// Whether `url` may be fetched: http or https, no credentials, a host the policy
    /// allows, and when the host is an address, one it may reach.
    pub fn check(&self, url: &Url) -> Result<(), FetchError> {
        match url.scheme() {
            "http" | "https" => {}
            other => return Err(FetchError::Scheme(other.to_owned())),
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(FetchError::Credentials);
        }
        let host = url
            .host_str()
            .ok_or_else(|| FetchError::NotUrl(url.to_string()))?;
        match self.policy.host(host) {
            HostRule::Allowed => {}
            HostRule::Denied(rule) => {
                return Err(FetchError::Denied {
                    host: host.to_owned(),
                    rule,
                });
            }
            HostRule::NotAllowed => return Err(FetchError::NotAllowed(host.to_owned())),
        }
        // The URL parser has already read decimal, octal and hex spellings of addresses.
        let address = match url.host() {
            Some(url::Host::Ipv4(ip)) => Some(IpAddr::V4(ip)),
            Some(url::Host::Ipv6(ip)) => Some(IpAddr::V6(ip)),
            _ => None,
        };
        if let Some(ip) = address
            && !reachable(ip, self.policy.private_ok(host))
        {
            return Err(FetchError::Private(format!(
                "{ip} is a private address (this computer, a local network or cloud metadata), \
                 which web_fetch does not reach"
            )));
        }
        Ok(())
    }

    /// Reads the body of `response`, up to the size limit, as text.
    async fn read(&self, url: Url, mut response: reqwest::Response) -> Result<Page, FetchError> {
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let mut body = Vec::new();
        let mut cut = false;
        while let Some(chunk) = response.chunk().await.map_err(|e| failed(&e))? {
            let room = self.limits.max_bytes.saturating_sub(body.len());
            if chunk.len() > room {
                body.extend_from_slice(chunk.get(..room).unwrap_or_default());
                cut = true;
                break;
            }
            body.extend_from_slice(&chunk);
        }
        let kind = kind(content_type.as_deref(), &body).ok_or_else(|| {
            FetchError::NotText(content_type.unwrap_or_else(|| "binary".to_owned()))
        })?;
        let (text, title) =
            tokio::task::spawn_blocking(move || (readable(kind, &body), title(kind, &body)))
                .await
                .map_err(|e| FetchError::Failed(e.to_string()))?;
        Ok(Page {
            url: url.to_string(),
            title,
            text,
            cut,
        })
    }
}

/// What went wrong, naming a refused private address when that is why.
fn failed(error: &reqwest::Error) -> FetchError {
    let mut source: Option<&dyn std::error::Error> = Some(error);
    while let Some(err) = source {
        if let Some(refused) = err.downcast_ref::<Refused>() {
            return FetchError::Private(refused.to_string());
        }
        source = err.source();
    }
    if error.is_timeout() {
        return FetchError::Failed("the connection timed out".to_owned());
    }
    FetchError::Failed(error.to_string())
}
