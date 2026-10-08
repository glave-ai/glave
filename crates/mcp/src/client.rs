//! glave's side of a connection: who it is, and hearing when a server's tools change,
//! whichever protocol version the server speaks. Roots are left out: MCP 2026-07-28 retires them.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rmcp::ClientHandler;
use rmcp::model::{
    ClientCapabilities, ClientConfig, Implementation, ServerNotification, SubscriptionFilter,
};
use rmcp::service::{NotificationContext, Peer, RoleClient};

/// Marks the tool listing stale when the server says its tools changed.
pub(crate) struct Client {
    stale: Arc<AtomicBool>,
}

impl Client {
    pub(crate) fn new(stale: Arc<AtomicBool>) -> Self {
        Self { stale }
    }
}

impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("glave", env!("CARGO_PKG_VERSION")),
        )
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

/// A server on MCP 2026-07-28 says so only on a subscription, which this keeps open until the
/// connection closes. Servers that cannot are left alone: their listing is kept as it is.
pub(crate) async fn watch_tools(peer: Peer<RoleClient>, stale: Arc<AtomicBool>) {
    let changes = SubscriptionFilter::builder().tools_list_changed().build();
    let Ok(mut subscription) = peer.listen(changes).await else {
        return;
    };
    while let Ok(Some(notification)) = subscription.next().await {
        if matches!(
            notification,
            ServerNotification::ToolListChangedNotification(_)
        ) {
            stale.store(true, Ordering::SeqCst);
        }
    }
}
