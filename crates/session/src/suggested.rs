//! The connections glave suggests (`specs/session.md`, "Connections"): each off until the
//! person turns it on, which adds it to their own settings, and theirs from then on. One today:
//! searching the web, at Exa's hosted MCP server, which works with no key. Its searches go to
//! Exa, so it is the person's to turn on, never on by default. glave's starter list (#679)
//! joins it here, until an organization's list of apps carries it (#633).

use crate::protocol::{ConnectionInfo, ConnectionStatus, Suggested};
use crate::words::Sentence;

/// Exa's hosted MCP server, offering only its search: glave's own `web_fetch` reads the pages.
const SEARCH_URL: &str = "https://mcp.exa.ai/mcp?tools=web_search_exa";

/// A connection glave suggests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// What the person's settings call it once it is on, unless they call another server that.
    pub name: &'static str,
    /// Its MCP server's address, which `connection/add` takes.
    pub url: String,
    /// The one tool its server offers, which searches the web: each call is told to the person
    /// as what was searched.
    pub tool: &'static str,
    title: Sentence,
    detail: Sentence,
}

impl Suggestion {
    /// Searching the web, at Exa's hosted server unless `url` puts another in its place, as
    /// tests do, which never reach Exa.
    pub fn search(url: Option<String>) -> Self {
        Self {
            name: "search",
            url: url.unwrap_or_else(|| SEARCH_URL.to_owned()),
            tool: "web_search_exa",
            title: Sentence::SearchTheWeb,
            detail: Sentence::SearchTheWebDetail,
        }
    }

    /// What it is for, in the server's own words.
    pub fn suggested(&self) -> Suggested {
        Suggested {
            title: self.title.clone().into(),
            detail: self.detail.clone().into(),
        }
    }

    /// It as `connection/list` lists it, `status` as the person's settings and sign-ins say.
    pub fn listed(&self, status: ConnectionStatus) -> ConnectionInfo {
        ConnectionInfo {
            url: self.url.clone(),
            name: Some(self.name.to_owned()),
            status,
            suggested: Some(self.suggested()),
            usage: None,
        }
    }
}
