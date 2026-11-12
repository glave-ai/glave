//! What the web tools may reach: whether fetching is on, which hosts (allow and deny patterns,
//! deny wins), and whether private addresses are reachable.

use crate::host::{Host, HostPattern};

/// What the web tools may reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebPolicy {
    /// Whether `web_fetch` is offered.
    pub fetch: bool,
    /// Hosts that may be fetched; every host when empty.
    pub allow: Vec<HostPattern>,
    /// Hosts that may never be fetched, each at its port or at every port. A deny wins over an
    /// allow.
    pub deny: Vec<HostPattern>,
    /// Whether any host may lead to a private address (this computer, the local network),
    /// though never to cloud metadata. Off, only a host `allow` names exactly, with no `*`,
    /// may, and it is then fetched only at the ports those entries name: a lookup sees no
    /// port, so `*` beside `localhost:3000` must not let `localhost` be reached at another.
    pub private_addresses: bool,
}

impl Default for WebPolicy {
    /// Fetching on, every public host, no private addresses.
    fn default() -> Self {
        Self {
            fetch: true,
            allow: Vec::new(),
            deny: Vec::new(),
            private_addresses: false,
        }
    }
}

/// Why the policy refuses a host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostRule {
    Allowed,
    /// A deny pattern names it.
    Denied(String),
    /// An allow list is set and names none of it.
    NotAllowed,
}

impl WebPolicy {
    /// Whether `host`, as a URL names it, may be fetched at `port`.
    pub fn host(&self, host: &str, port: u16) -> HostRule {
        let host = Host::of_url(host);
        if let Some(rule) = self.deny.iter().find(|deny| deny.admits(&host, port)) {
            return HostRule::Denied(rule.to_string());
        }
        let mut exact = self.exactly(&host).peekable();
        let allowed = if exact.peek().is_some() && !self.private_addresses {
            exact.any(|allow| allow.admits(&host, port))
        } else {
            self.allow.is_empty() || self.allow.iter().any(|allow| allow.admits(&host, port))
        };
        if allowed {
            HostRule::Allowed
        } else {
            HostRule::NotAllowed
        }
    }

    /// Whether `host` may lead to a private address, at whatever port [`WebPolicy::host`]
    /// allows it.
    pub fn private_ok(&self, host: &str) -> bool {
        self.private_addresses || self.exactly(&Host::of_url(host)).next().is_some()
    }

    /// The `allow` entries that name `host` exactly.
    fn exactly<'a>(&'a self, host: &'a Host) -> impl Iterator<Item = &'a HostPattern> {
        self.allow
            .iter()
            .filter(move |allow| allow.is_exact() && allow.names(host))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patterns(texts: &[&str]) -> Vec<HostPattern> {
        texts
            .iter()
            .map(|p| HostPattern::parse(p).unwrap())
            .collect()
    }

    #[test]
    fn deny_wins_and_an_allow_list_narrows() {
        let policy = WebPolicy {
            allow: patterns(&["*.example.com", "docs.rs", "wiki.corp:8443"]),
            deny: patterns(&["secret.example.com"]),
            ..WebPolicy::default()
        };
        assert_eq!(policy.host("a.example.com", 443), HostRule::Allowed);
        assert_eq!(policy.host("DOCS.RS.", 8080), HostRule::Allowed);
        assert_eq!(policy.host("wiki.corp", 8443), HostRule::Allowed);
        assert_eq!(policy.host("wiki.corp", 443), HostRule::NotAllowed);
        assert_eq!(
            policy.host("secret.example.com", 8443),
            HostRule::Denied("secret.example.com".into())
        );
        assert_eq!(policy.host("crates.io", 443), HostRule::NotAllowed);
        let closed = WebPolicy {
            deny: patterns(&["*"]),
            ..WebPolicy::default()
        };
        assert!(matches!(closed.host("docs.rs", 443), HostRule::Denied(_)));
    }

    #[test]
    fn only_hosts_named_exactly_may_be_private() {
        let policy = WebPolicy {
            allow: patterns(&["127.0.0.1", "*.corp", "[::1]"]),
            ..WebPolicy::default()
        };
        assert!(policy.private_ok("127.0.0.1"));
        assert!(policy.private_ok("[::1]"));
        assert!(!policy.private_ok("wiki.corp"));
        assert!(!WebPolicy::default().private_ok("localhost"));
        let open = WebPolicy {
            private_addresses: true,
            ..WebPolicy::default()
        };
        assert!(open.private_ok("localhost"));
    }

    #[test]
    fn a_host_that_may_be_private_is_fetched_only_at_the_ports_named_for_it() {
        for named in ["localhost", "127.0.0.1"] {
            let policy = WebPolicy {
                allow: patterns(&["*", &format!("{named}:3000")]),
                ..WebPolicy::default()
            };
            assert!(policy.private_ok(named), "{named}");
            assert_eq!(policy.host(named, 3000), HostRule::Allowed, "{named}");
            assert_eq!(policy.host(named, 6379), HostRule::NotAllowed, "{named}");
            // With private addresses on, an entry gives no host a reach it didn't have.
            let open = WebPolicy {
                private_addresses: true,
                ..policy
            };
            assert_eq!(open.host(named, 6379), HostRule::Allowed, "{named}");
        }
    }
}
