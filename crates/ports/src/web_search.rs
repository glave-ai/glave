//! What glave's `web_search` asks of a model provider, and what comes back (`specs/session.md`,
//! "Web search"): a query and how many results, and the pages found, each a title, an address
//! and a few words from it, with what the search cost when its service says.

use glave_event::Money;

/// One search: what to look for, and how many results at most.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchAsk {
    pub query: String,
    pub count: u8,
}

/// One page a search found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub title: String,
    pub url: String,
    /// A few words from the page, as the search service gives them.
    pub snippet: String,
}

/// What a search found, in the service's order, and what it cost when the service says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Searched {
    pub hits: Vec<Hit>,
    pub cost: Option<Money>,
}
