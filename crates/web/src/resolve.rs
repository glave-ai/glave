//! The fetch client's name lookups: glave's one [`Lookup`], keeping the addresses the host
//! policy allows, so a host cannot resolve to a public address when checked and a private one
//! when connected (DNS rebinding); a host that leads to none is refused in `web_fetch`'s words.

use std::fmt;
use std::sync::Arc;

use glave_ports::host::Lookup;
use glave_ports::settings::WebPolicy;

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

/// The lookups `policy` allows: private addresses only for the hosts it lets be private.
pub(crate) fn guarded(policy: Arc<WebPolicy>) -> Lookup<Refused> {
    Lookup::new(
        move |host| policy.private_ok(host),
        |host| Refused {
            host: host.to_owned(),
        },
    )
}
