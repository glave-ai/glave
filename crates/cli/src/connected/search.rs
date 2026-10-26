//! Web search (`specs/session.md`, "Web search"): under the hood a connection of the person's,
//! Exa's hosted MCP server offering only its search, which the person turns on and off as a
//! setting. On, its server is in their own `mcp.json`, at its address; off, it is not. Turning
//! it off takes out only the server glave wrote; one the person set up there themselves stays.
//! Its searches go to Exa, so it is never on until they turn it on.

use glave_mcp::address;
use glave_session::Searching;
use glave_session::protocol::{ApiError, code};

use super::Connected;
use crate::mcp::Removed;

/// Exa's hosted MCP server, offering only its search: glave's own `web_fetch` reads the pages.
const EXA: &str = "https://mcp.exa.ai/mcp?tools=web_search_exa";

/// Web search, as glave keeps it for the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WebSearch {
    /// What the person's settings call it once it is on, unless they call another server that.
    pub(crate) name: &'static str,
    /// Its MCP server's address.
    pub(crate) url: String,
    /// Its server's one tool, which searches the web: each call is told to the person as what
    /// was searched.
    pub(crate) tool: &'static str,
}

impl WebSearch {
    /// At Exa's hosted server, unless `url` puts a stand-in in its place, as tests do, which
    /// never reach Exa.
    pub(crate) fn at(url: Option<String>) -> Self {
        Self {
            name: "search",
            url: url.unwrap_or_else(|| EXA.to_owned()),
            tool: "web_search_exa",
        }
    }
}

impl Connected {
    /// Web search, when the server at `url` is its.
    pub(crate) fn search_at(&self, url: &str) -> Option<&WebSearch> {
        let search = self.search.as_ref()?;
        let at = address(url)?;
        (address(&search.url).as_deref() == Some(at.as_str())).then_some(search)
    }

    /// Web search's address, as glave keeps addresses.
    pub(super) fn search_address(&self) -> Option<String> {
        address(&self.search.as_ref()?.url)
    }

    /// Whether the person's agent searches the web: on when their settings have a server at
    /// its address, under the name they give it.
    pub(super) async fn searching(&self) -> Result<Option<Searching>, ApiError> {
        let Some(search) = &self.search else {
            return Ok(None);
        };
        let url = address(&search.url).unwrap_or_default();
        let remotes = self.remotes().await;
        Ok(Some(match remotes.get(&url) {
            Some(remote) => Searching {
                on: true,
                name: remote.name.clone(),
            },
            None => Searching {
                on: false,
                name: search.name.to_owned(),
            },
        }))
    }

    /// Turns web search on or off in the person's own settings: how it is then.
    pub(super) async fn searching_set(&self, on: bool) -> Result<Option<Searching>, ApiError> {
        let Some(search) = &self.search else {
            return Ok(None);
        };
        if on {
            let name = self
                .writer
                .add(&self.home, search.name, &search.url)
                .await?;
            return Ok(Some(Searching { on, name }));
        }
        if self.writer.remove(&self.home, &search.url).await? == Removed::Theirs {
            return Err(ApiError::new(
                code::NOT_AVAILABLE,
                "you set up web search yourself in your own settings, so glave leaves it there: \
                 change it there",
            ));
        }
        self.searching().await
    }
}
