//! glave's side of a connection: who it is, the folder it works for, and hearing when a
//! server's tools change, whichever protocol version the server speaks.

#![expect(
    deprecated,
    reason = "MCP 2026-07-28 deprecates roots (SEP-2577), but servers on 2025-11-25 still ask"
)]

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rmcp::ClientHandler;
use rmcp::model::{
    ClientCapabilities, ClientConfig, Implementation, ListRootsResult, Root, RootsCapabilities,
    ServerNotification, SubscriptionFilter,
};
use rmcp::service::{NotificationContext, Peer, RequestContext, RoleClient, Subscription};

/// Marks the tool listing stale when the server says its tools changed, and tells a server
/// that asks which folder it works for.
pub(crate) struct Client {
    stale: Arc<AtomicBool>,
    /// The project folder the server works for, as a `file://` root: none for a server that
    /// works for every project, which is told of no folder.
    roots: Vec<Root>,
}

impl Client {
    /// A client for a server that works for the project at `folder`, if it is one project's.
    pub(crate) fn new(stale: Arc<AtomicBool>, folder: Option<&Path>) -> Self {
        let roots = folder.and_then(root).into_iter().collect();
        Self { stale, roots }
    }

    /// What a server that asks for the client's roots is told.
    pub(crate) fn roots(&self) -> ListRootsResult {
        ListRootsResult::new(self.roots.clone())
    }
}

/// The project at `folder` as a root, named after its folder; none when it cannot be written
/// as a `file://` URL (a relative path).
fn root(folder: &Path) -> Option<Root> {
    let url = url::Url::from_directory_path(folder).ok()?;
    let root = Root::new(url.as_str());
    Some(match folder.file_name().and_then(|name| name.to_str()) {
        Some(name) => root.with_name(name),
        None => root,
    })
}

impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        let mut capabilities = ClientCapabilities::default();
        if !self.roots.is_empty() {
            // The roots never change while connected: a project's servers start again in a
            // project of their own.
            capabilities.roots = Some(RootsCapabilities::default());
        }
        ClientConfig::new(
            capabilities,
            Implementation::new("glave", env!("CARGO_PKG_VERSION")),
        )
    }

    /// A server on the older protocol asks at any time; on MCP 2026-07-28 it asks inside a
    /// call (`connection.rs`).
    fn list_roots(
        &self,
        _context: RequestContext<RoleClient>,
    ) -> impl Future<Output = Result<ListRootsResult, rmcp::ErrorData>> + Send + '_ {
        std::future::ready(Ok(self.roots()))
    }

    /// A server on the older protocol says so at any time.
    fn on_tool_list_changed(
        &self,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + Send + '_ {
        self.stale.store(true, Ordering::SeqCst);
        std::future::ready(())
    }
}

/// A server on MCP 2026-07-28 says so only on a subscription. This opens one, returning once the
/// server has acknowledged it, so no change made after the connection opens goes unheard. Servers
/// that cannot are left alone: their listing is kept as it is.
pub(crate) async fn subscribe(peer: &Peer<RoleClient>) -> Option<Subscription> {
    let changes = SubscriptionFilter::builder().tools_list_changed().build();
    peer.listen(changes).await.ok()
}

/// Marks the listing stale on every change heard on `subscription`, until the connection closes.
pub(crate) async fn watch_tools(mut subscription: Subscription, stale: Arc<AtomicBool>) {
    while let Ok(Some(notification)) = subscription.next().await {
        if matches!(
            notification,
            ServerNotification::ToolListChangedNotification(_)
        ) {
            stale.store(true, Ordering::SeqCst);
        }
    }
}
