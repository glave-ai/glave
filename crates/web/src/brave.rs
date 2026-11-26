//! Brave's Search API, the service glave's `web_search` searches through (`specs/session.md`,
//! "Web search"): `GET https://api.search.brave.com/res/v1/web/search?q=…&count=…` with the
//! key in `X-Subscription-Token`, its web results' titles, addresses and descriptions read back.
//! The key is the server's (`GLAVE_SEARCH_KEY`), sent to Brave alone and never shown: not in
//! `Debug`, an error, or a log line. Brave's Search plan bills $5 for 1,000 searches, and none it
//! refused, a rate limit's among them (<https://brave.com/search/api/>, read 2026-11-14).

use std::fmt;
use std::time::Duration;

use glave_event::{Currency, Money, Nanos, Secret};
use glave_ports::{
    Completion, Delta, Failure, Hit, ModelProvider, ModelRequest, ProviderError, Purpose,
    SearchAsk, SearchResults,
};
use reqwest::header::ACCEPT;
use reqwest::{Client, Url};
use serde_json::Value;

/// Brave's web search.
const BRAVE: &str = "https://api.search.brave.com/res/v1/web/search";

/// What one search costs on Brave's Search plan: $5 for 1,000, in billionths of a dollar.
const PRICE_NANOS: i64 = 5_000_000;

/// The longest a search may take, answer and all.
const TIMEOUT: Duration = Duration::from_secs(30);

/// The most bytes of an answer read: Brave's are tens of kilobytes.
const MOST_BYTES: usize = 2 * 1024 * 1024;

/// The most characters of an error's body kept, which says why Brave refused.
const WHY_CHARS: usize = 300;

/// Searches the web at Brave.
pub struct Brave {
    client: Client,
    url: Url,
    key: Secret,
}

impl fmt::Debug for Brave {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Brave")
            .field("url", &glave_ports::host::shown(self.url.as_str()))
            .finish_non_exhaustive()
    }
}

/// Why Brave could not be set up.
#[derive(Debug, thiserror::Error)]
pub enum BraveError {
    #[error("the address web search is reached at is not an http or https URL")]
    Url,
    #[error("could not set up web search's client: {0}")]
    Client(String),
}

impl Brave {
    /// Brave at its own address, or at `url` in its place, as a stand-in for tests and trying
    /// glave by hand, which never reach Brave.
    pub fn new(key: Secret, url: Option<&str>) -> Result<Self, BraveError> {
        let at = url.unwrap_or(BRAVE);
        let url = Url::parse(at).map_err(|_| BraveError::Url)?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(BraveError::Url);
        }
        let client = Client::builder()
            .timeout(TIMEOUT)
            .build()
            .map_err(|error| BraveError::Client(error.to_string()))?;
        Ok(Self { client, url, key })
    }

    /// What one search costs.
    pub fn price() -> Money {
        Money {
            nanos: Nanos::new(PRICE_NANOS),
            currency: Currency::USD,
        }
    }

    async fn searched(&self, ask: &SearchAsk) -> Result<SearchResults, ProviderError> {
        let count = ask.count.clamp(1, SearchAsk::MOST_RESULTS).to_string();
        let mut url = self.url.clone();
        url.query_pairs_mut()
            .append_pair("q", &ask.query)
            .append_pair("count", &count);
        let sent = self
            .client
            .get(url)
            .header(ACCEPT, "application/json")
            .header("X-Subscription-Token", self.key.expose())
            .send()
            .await;
        let mut response = sent.map_err(transport)?;
        let status = response.status();
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport)? {
            body.extend_from_slice(&chunk);
            if body.len() > MOST_BYTES {
                return Err(ProviderError::Protocol(
                    "web search's answer was too large".to_owned(),
                ));
            }
        }
        if !status.is_success() {
            let why: String = String::from_utf8_lossy(&body)
                .chars()
                .take(WHY_CHARS)
                .collect();
            return Err(ProviderError::Http {
                status: status.as_u16(),
                body: why,
            });
        }
        let answer: Value = serde_json::from_slice(&body)
            .map_err(|_| ProviderError::Protocol("web search's answer was not JSON".to_owned()))?;
        Ok(SearchResults {
            hits: hits(&answer),
            cost: None,
        })
    }
}

/// Why Brave could not be reached, without the address it was sent to, which holds the query.
fn transport(error: reqwest::Error) -> ProviderError {
    // Never reached Brave (a connection or a name that failed): nothing was billed.
    if error.is_connect() {
        return ProviderError::Unserved {
            failure: Failure::Unreachable,
            reason: error.without_url().to_string(),
        };
    }
    if error.is_timeout() {
        return ProviderError::Timeout {
            seconds: TIMEOUT.as_secs(),
        };
    }
    ProviderError::Transport(error.without_url().to_string())
}

/// The web results of Brave's `answer`, in its order: those with a title and an address.
fn hits(answer: &Value) -> Vec<Hit> {
    let results = answer
        .pointer("/web/results")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    results
        .iter()
        .filter_map(|result| {
            let text = |key: &str| result.get(key).and_then(Value::as_str).map(plain);
            Some(Hit {
                title: text("title")?,
                url: result.get("url")?.as_str()?.to_owned(),
                snippet: text("description").unwrap_or_default(),
            })
        })
        .collect()
}

/// `html`, a title or description as Brave gives it, as plain text: its tags (`<strong>` around
/// the words searched for) dropped, its character references read, and kept to one line of
/// what a person sees, so no reference can put a line end or a control character in it.
fn plain(html: &str) -> String {
    glave_event::one_line(&crate::title::references(&crate::text::strip_tags(html)))
}

/// Brave answers searches alone: never a model's call, nor a media tool's.
impl ModelProvider for Brave {
    fn complete(
        &self,
        request: &ModelRequest,
        _: &mut (dyn FnMut(Delta) + Send),
    ) -> impl Future<Output = Result<Completion, ProviderError>> + Send {
        std::future::ready(Err(ProviderError::Unserved {
            failure: Failure::Rejected,
            reason: format!("web search serves no {}", request.purpose.id()),
        }))
    }

    async fn search(
        &self,
        _: &ModelRequest,
        ask: &SearchAsk,
    ) -> Result<SearchResults, ProviderError> {
        self.searched(ask).await
    }

    fn serves(&self, purpose: Purpose) -> bool {
        purpose == Purpose::WebSearch
    }
}
