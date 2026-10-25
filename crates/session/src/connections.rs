//! Connections (`specs/session.md`, "Connections"): the person on this computer signs in to a
//! remote MCP server in the browser, as MCP's authorization says, and the agent uses it as them
//! from then on, their token asked for at each request. The host keeps the sign-ins sealed and
//! hands their tokens to the servers; the server decides who may ask, and is where the browser
//! comes back to.

use glave_event::Secret;
use glave_ports::{BlobStore, BoxFuture, Caller, EventLog, ModelProvider};

use crate::protocol::{ApiError, ConnectionInfo, Reply, SearchInfo, SignInLink, code};
use crate::providers::theirs;
use crate::server::Server;
use crate::words::Sentence;

/// Where the browser comes back to from a sign-in, on the server's own address.
pub(crate) const CALLBACK: &str = "/connect/callback";

/// What the browser came back with from the authorization server: the sign-in's `state`, and
/// its code, or why there is none.
#[derive(Debug, Clone)]
pub struct Returned {
    pub state: String,
    /// The code to trade for a token.
    pub code: Option<Secret>,
    /// The authorization server that sent it, when it named itself (RFC 9207).
    pub issuer: Option<String>,
    /// Why there is no code: `access_denied` when the person declined.
    pub error: Option<String>,
}

impl Returned {
    /// What a callback's `query` says.
    fn read(query: &str) -> Self {
        let url = reqwest::Url::parse(&format!("http://callback.invalid/?{query}")).ok();
        let field = |name: &str| {
            url.as_ref().and_then(|url| {
                url.query_pairs()
                    .find(|(field, _)| field == name)
                    .map(|(_, value)| value.into_owned())
            })
        };
        Self {
            state: field("state").unwrap_or_default(),
            code: field("code").map(Secret::new),
            issuer: field("iss"),
            error: field("error"),
        }
    }
}

/// What keeps a person's sign-ins to their connections: the host, which also hands their
/// tokens to the servers.
pub trait Connections: Send + Sync {
    /// Starts signing in to the person's remote MCP server `server`, by its name in their
    /// settings or its address, the browser to come back to `redirect`. With no `redirect`, it
    /// starts nothing and answers none once `server` is theirs, which the server then says.
    fn start(
        &self,
        server: String,
        redirect: Option<String>,
    ) -> BoxFuture<'_, Result<Option<SignInLink>, ApiError>>;

    /// Waits a little for the sign-in `flow`: the connection once signed in to, none while
    /// it waits.
    fn poll(&self, flow: String) -> BoxFuture<'_, Result<Option<ConnectionInfo>, ApiError>>;

    /// Finishes the sign-in the browser came back from: what to tell the person there.
    fn finish(&self, returned: Returned) -> BoxFuture<'_, String>;

    /// The connections signed in to, then the person's remote servers that ask them to sign
    /// in and are not signed in to; never the web search's.
    fn list(&self) -> BoxFuture<'_, Result<Vec<ConnectionInfo>, ApiError>>;

    /// Removes the sign-in to `server`, by its name in the person's settings or its address,
    /// destroying its tokens.
    fn remove(&self, server: String) -> BoxFuture<'_, Result<(), ApiError>>;

    /// Whether the person's agent searches the web: none where it is not theirs to choose.
    fn search(&self) -> BoxFuture<'_, Result<Option<Searching>, ApiError>>;

    /// Turns web search on or off in the person's own settings: how it is then.
    fn set_search(&self, on: bool) -> BoxFuture<'_, Result<Option<Searching>, ApiError>>;
}

/// Whether the person's agent searches the web, as their own settings say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Searching {
    pub on: bool,
    /// What their settings call its server, which its calls are counted by.
    pub name: String,
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The host's sign-ins to connections, for the person on this computer only.
    fn connections_for(&self, caller: &Caller) -> Result<&dyn Connections, ApiError> {
        let connections = self.inner.connections.as_deref();
        let elsewhere = Sentence::ConnectionsOnItsComputer;
        theirs(caller, connections, Sentence::ConnectionsNotHere, elsewhere)
    }

    /// Answers `connection/start`. The browser comes back to where people reach the server
    /// from elsewhere when it has such a place, else to its address on this computer. A server
    /// that does not listen over HTTP has none, nor one that listens beyond loopback with no
    /// such place: each says so, once the server is the caller's.
    pub(crate) async fn start_connection(
        &self,
        caller: &Caller,
        server: String,
    ) -> Result<Reply, ApiError> {
        let connections = self.connections_for(caller)?;
        let origin = self.inner.origin.as_ref();
        let redirect = self.inner.reached.get().and_then(|here| {
            let base = origin.map(|origin| origin.as_str().to_owned());
            let base = base.or_else(|| here.clone())?;
            Some(format!("{base}{CALLBACK}"))
        });
        match connections.start(server, redirect).await? {
            Some(sign_in) => Ok(Reply::SignIn { sign_in }),
            None if self.inner.reached.get().is_none() => Err(ApiError::new(
                code::NOT_AVAILABLE,
                Sentence::SignInNeedsServer,
            )),
            None => Err(ApiError::new(
                code::NOT_AVAILABLE,
                Sentence::SignInNoWayBack,
            )),
        }
    }

    /// Answers `connection/poll`.
    pub(crate) async fn poll_connection(
        &self,
        caller: &Caller,
        flow: String,
    ) -> Result<Reply, ApiError> {
        let connections = self.connections_for(caller)?;
        Ok(match connections.poll(flow.clone()).await? {
            Some(mut connection) => {
                self.used(caller, std::slice::from_mut(&mut connection))
                    .await;
                Reply::Connection { connection }
            }
            None => Reply::Waiting { flow },
        })
    }

    /// Answers `connection/list`.
    pub(crate) async fn list_connections(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let mut connections = self.connections_for(caller)?.list().await?;
        self.used(caller, &mut connections).await;
        Ok(Reply::Connections { connections })
    }

    /// Answers `search/get`: never the person's to choose where this server keeps no
    /// connections of theirs.
    pub(crate) async fn get_search(&self, caller: &Caller) -> Result<Reply, ApiError> {
        if self.inner.connections.is_none() {
            return Ok(Reply::Search {
                search: SearchInfo::Never,
            });
        }
        let searching = self.connections_for(caller)?.search().await?;
        let search = self.search_info(caller, searching).await;
        Ok(Reply::Search { search })
    }

    /// Answers `search/set`.
    pub(crate) async fn set_search(&self, caller: &Caller, on: bool) -> Result<Reply, ApiError> {
        let searching = self.connections_for(caller)?.set_search(on).await?;
        let search = self.search_info(caller, searching).await;
        Ok(Reply::Search { search })
    }

    /// Answers `connection/remove`.
    pub(crate) async fn remove_connection(
        &self,
        caller: &Caller,
        server: String,
    ) -> Result<Reply, ApiError> {
        self.connections_for(caller)?.remove(server).await?;
        Ok(Reply::Done {})
    }

    /// What to tell the browser that came back from a sign-in with `query`. It carries no
    /// sign-in of glave's own: the sign-in's `state`, which only that browser was given, names
    /// what it finishes.
    pub(crate) async fn finish_connection(&self, query: &str) -> String {
        match self.inner.connections.as_deref() {
            Some(connections) => connections.finish(Returned::read(query)).await,
            None => Sentence::ConnectionsNotHere.english().to_owned(),
        }
    }

    /// Records that the server listens over HTTP, and its address on this computer, `http://`
    /// and its loopback listener's, when it has one: where a browser here comes back to from a
    /// sign-in.
    pub(crate) fn reached_at(&self, here: Option<String>) {
        let _ = self.inner.reached.set(here);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_callback_says_its_state_code_issuer_or_error() {
        let back = Returned::read("code=abc%2B1&state=s1&iss=https%3A%2F%2Fauth.example.com");
        assert_eq!(back.state, "s1");
        assert_eq!(back.code.as_ref().map(Secret::expose), Some("abc+1"));
        assert_eq!(back.issuer.as_deref(), Some("https://auth.example.com"));
        assert_eq!(back.error, None);
        let declined = Returned::read("error=access_denied&state=s2");
        assert_eq!(
            (declined.state.as_str(), declined.error.as_deref()),
            ("s2", Some("access_denied"))
        );
        assert!(declined.code.is_none());
        assert!(!format!("{back:?}").contains("abc"), "the code never shows");
    }
}
