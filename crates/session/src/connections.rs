//! Connections (`specs/session.md`, "Connections"): the person on this computer signs in to a
//! remote MCP server in the browser, as MCP's authorization says, and the agent uses it as them
//! from then on, their token asked for at each request. The host keeps the sign-ins sealed and
//! hands their tokens to the servers; the server decides who may ask, and is where the browser
//! comes back to. The connectors of glave's catalog (`connectors.rs`) are connected the same
//! way, by their ids, and listed with the catalog's words in the caller's language, each with
//! where the person's threads use it: everywhere, or only in one project of theirs
//! (`connections/connectors.rs`).

mod connectors;

use glave_event::Secret;
use glave_ports::{BlobStore, BoxFuture, Caller, EventLog, ModelProvider};

use crate::accounts::KeptConnector;
use crate::protocol::{
    ApiError, ConnectionInfo, ConnectorStatus, Reply, SearchInfo, SignInLink, Told, code,
};
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

    /// The connectors of glave's catalog the person may connect here, in any order: each
    /// account of one connected, with whether its sign-in works, where their threads use it and
    /// who it is signed in as, and each one not connected once, as `Available`.
    fn connectors(&self) -> BoxFuture<'_, Result<Vec<OfferedAccount>, ApiError>>;

    /// Connects `connector`, an account of a connector of glave's catalog, to be used where it
    /// says: at once when it asks no one to sign in, else by signing in as that account, the
    /// browser to come back to `redirect`. `another`, unnamed, it is a new account of a service
    /// connected already, named as it is kept: as who it is signed in as, where its service
    /// says, else the service's name and a number.
    fn connect(
        &self,
        connector: KeptConnector,
        another: bool,
        redirect: Option<String>,
    ) -> BoxFuture<'_, Result<Connecting, ApiError>>;

    /// Uses the connected account `connector` names where it says from now on.
    fn place(&self, connector: KeptConnector) -> BoxFuture<'_, Result<(), ApiError>>;

    /// Disconnects the account `connector` names, wherever it is used, destroying its sign-in.
    fn disconnect(&self, connector: KeptConnector) -> BoxFuture<'_, Result<(), ApiError>>;

    /// Calls the account `connector` names `name` from now on (none: unnamed), its sign-in
    /// with it.
    fn rename(
        &self,
        connector: KeptConnector,
        name: Option<String>,
    ) -> BoxFuture<'_, Result<(), ApiError>>;
}

/// An account of a connector of glave's catalog as the host offers it: connected or not, and
/// who it is signed in as, where its service said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferedAccount {
    pub connector: KeptConnector,
    pub status: ConnectorStatus,
    /// The person's own, shown only to them: `Acme Corp`.
    pub who: Option<String>,
}

impl OfferedAccount {
    /// `connector`, `status`, with no one it is known to be signed in as.
    pub fn new(connector: KeptConnector, status: ConnectorStatus) -> Self {
        Self {
            connector,
            status,
            who: None,
        }
    }
}

/// How connecting a connector went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Connecting {
    /// Connected: it asks no one to sign in.
    Connected,
    /// The person signs in on this page.
    SignIn(SignInLink),
    /// It asks for a sign-in, and the browser has nowhere to come back to.
    NoWayBack,
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

    /// Where the browser comes back to from a sign-in: where people reach the server from
    /// elsewhere when it has such a place, else its address on this computer; none where it
    /// does not listen over HTTP.
    fn redirect(&self) -> Option<String> {
        let origin = self.inner.origin.as_ref();
        self.inner.reached.get().and_then(|here| {
            let base = origin.map(|origin| origin.as_str().to_owned());
            let base = base.or_else(|| here.clone())?;
            Some(format!("{base}{CALLBACK}"))
        })
    }

    /// Why a sign-in cannot start: the server does not listen over HTTP, or listens beyond
    /// loopback with no address people reach it at.
    fn no_way_back(&self) -> ApiError {
        let said = match self.inner.reached.get() {
            None => Sentence::SignInNeedsServer,
            Some(_) => Sentence::SignInNoWayBack,
        };
        ApiError::new(code::NOT_AVAILABLE, said)
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
        match connections.start(server, self.redirect()).await? {
            Some(sign_in) => Ok(Reply::SignIn { sign_in }),
            None => Err(self.no_way_back()),
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

    /// Web search's keeper, when it is `caller`'s to choose: the person on this computer, where
    /// this server keeps connections of theirs.
    fn search_for(&self, caller: &Caller) -> Result<&dyn Connections, ApiError> {
        let connections = self.inner.connections.as_deref();
        let elsewhere = Sentence::SearchOnItsComputer;
        theirs(caller, connections, Sentence::SearchNotHere, elsewhere)
    }

    /// Answers `search/get`, counting this month's searches when `usage` asks: never the
    /// caller's to choose, with why, where `search/set` would refuse them.
    pub(crate) async fn get_search(&self, caller: &Caller, usage: bool) -> Result<Reply, ApiError> {
        let never = |reason: Told| Reply::Search {
            search: SearchInfo::Never { reason },
        };
        let connections = match self.search_for(caller) {
            Ok(connections) => connections,
            Err(refused) => return Ok(never(refused.message)),
        };
        let Some(searching) = connections.search().await? else {
            return Ok(never(Sentence::SearchNotHere.into()));
        };
        let search = self.search_info(caller, searching, usage).await;
        Ok(Reply::Search { search })
    }

    /// Answers `search/set`, with this month's searches, for Settings, which turned it.
    pub(crate) async fn set_search(&self, caller: &Caller, on: bool) -> Result<Reply, ApiError> {
        let Some(searching) = self.search_for(caller)?.set_search(on).await? else {
            return Err(ApiError::new(code::NOT_AVAILABLE, Sentence::SearchNotHere));
        };
        let search = self.search_info(caller, searching, true).await;
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
