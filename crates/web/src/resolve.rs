//! The fetch client's name lookups. Every address a host resolves to is checked, and only the
//! allowed ones are handed to the connection, so a host cannot resolve to a public address
//! when checked and a private one when connected (DNS rebinding).

use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

use crate::address::is_public;
use crate::policy::WebPolicy;

/// A host that leads only to addresses the policy refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Refused {
    pub(crate) host: String,
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} resolves only to private addresses (this computer or a local network), which \
             web_fetch does not reach",
            self.host
        )
    }
}

impl std::error::Error for Refused {}

/// Looks names up with the system resolver, keeping only the addresses `policy` allows.
pub(crate) struct Guarded {
    pub(crate) policy: Arc<WebPolicy>,
}

impl Resolve for Guarded {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        let private_ok = self.policy.private_ok(&host);
        Box::pin(async move {
            let found: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|addr| private_ok || is_public(addr.ip()))
                .collect();
            if found.is_empty() {
                return Err(Box::new(Refused { host }) as Box<dyn std::error::Error + Send + Sync>);
            }
            let addrs: Addrs = Box::new(found.into_iter());
            Ok(addrs)
        })
    }
}
