//! What the web tools may reach: whether fetching is on, which hosts (allow and deny globs,
//! deny wins), and whether private addresses are reachable.

use std::fmt;

use thiserror::Error;

/// A host pattern: a host name or address, where `*` stands for any run of characters.
/// `example.com` is that host alone; `*.example.com` its subdomains; `*` every host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Domain(String);

/// A pattern glave cannot read.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{pattern:?} is not a host pattern: {why}")]
pub struct BadDomain {
    pub pattern: String,
    pub why: &'static str,
}

impl Domain {
    pub fn parse(pattern: &str) -> Result<Self, BadDomain> {
        let bad = |why| BadDomain {
            pattern: pattern.to_owned(),
            why,
        };
        let text = normal(pattern.trim());
        if text.is_empty() {
            return Err(bad("it is empty"));
        }
        if text.contains("://") || text.contains('/') {
            return Err(bad("name the host only, such as docs.rs or *.example.com"));
        }
        if !text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-.*:_".contains(c))
        {
            return Err(bad("a host has only letters, digits, dots, dashes and *"));
        }
        Ok(Self(text))
    }

    /// Whether `host` (lowercase) matches.
    pub fn matches(&self, host: &str) -> bool {
        glob(self.0.as_bytes(), host.as_bytes())
    }

    /// Whether the pattern names one host exactly, with no `*`.
    fn exact(&self) -> bool {
        !self.0.contains('*')
    }
}

impl fmt::Display for Domain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// `*`-only glob matching, iterative so no pattern can make it slow.
fn glob(pattern: &[u8], text: &[u8]) -> bool {
    let (mut p, mut t) = (0_usize, 0_usize);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        match (pattern.get(p), text.get(t)) {
            (Some(b'*'), _) => {
                star = Some((p, t));
                p = p.saturating_add(1);
            }
            (Some(a), Some(b)) if a == b => {
                p = p.saturating_add(1);
                t = t.saturating_add(1);
            }
            _ => match star {
                Some((sp, st)) => {
                    p = sp.saturating_add(1);
                    t = st.saturating_add(1);
                    star = Some((sp, t));
                }
                None => return false,
            },
        }
    }
    pattern
        .get(p..)
        .is_some_and(|rest| rest.iter().all(|c| *c == b'*'))
}

/// What the web tools may reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebPolicy {
    /// Whether `web_fetch` is offered.
    pub fetch: bool,
    /// Hosts that may be fetched; every host when empty.
    pub allow: Vec<Domain>,
    /// Hosts that may never be fetched. A deny wins over an allow.
    pub deny: Vec<Domain>,
    /// Whether any host may lead to a private address (this computer, the local network,
    /// cloud metadata). Off, only a host `allow` names exactly, with no `*`, may.
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
    /// Whether `host` may be fetched by name.
    pub fn host(&self, host: &str) -> HostRule {
        let host = normal(host);
        if let Some(rule) = self.deny.iter().find(|deny| deny.matches(&host)) {
            return HostRule::Denied(rule.to_string());
        }
        if self.allow.is_empty() || self.allow.iter().any(|allow| allow.matches(&host)) {
            HostRule::Allowed
        } else {
            HostRule::NotAllowed
        }
    }

    /// Whether `host` may lead to a private address.
    pub fn private_ok(&self, host: &str) -> bool {
        let host = normal(host);
        self.private_addresses
            || self
                .allow
                .iter()
                .any(|allow| allow.exact() && allow.matches(&host))
    }
}

/// A host as patterns see it: lowercase, without a final dot or IPv6 brackets.
fn normal(host: &str) -> String {
    host.trim_end_matches('.')
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn domains(patterns: &[&str]) -> Vec<Domain> {
        patterns.iter().map(|p| Domain::parse(p).unwrap()).collect()
    }

    #[test]
    fn globs_match_hosts() {
        let d = |p: &str| Domain::parse(p).unwrap();
        assert!(d("docs.rs").matches("docs.rs"));
        assert!(!d("docs.rs").matches("evil-docs.rs"));
        assert!(!d("docs.rs").matches("docs.rs.evil.com"));
        assert!(d("*.example.com").matches("a.b.example.com"));
        assert!(!d("*.example.com").matches("example.com"));
        assert!(!d("*.example.com").matches("example.com.evil"));
        assert!(d("*").matches("anything.at.all"));
        assert!(d("Docs.RS.").matches("docs.rs"));
        assert!(glob(b"a*b*c", b"aXXbYYc") && !glob(b"a*b*c", b"aXXbYY"));
        for bad in ["", "https://docs.rs", "docs.rs/path", "dö.cs"] {
            assert!(Domain::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn deny_wins_and_an_allow_list_narrows() {
        let policy = WebPolicy {
            allow: domains(&["*.example.com", "docs.rs"]),
            deny: domains(&["secret.example.com"]),
            ..WebPolicy::default()
        };
        assert_eq!(policy.host("a.example.com"), HostRule::Allowed);
        assert_eq!(policy.host("DOCS.RS."), HostRule::Allowed);
        assert_eq!(
            policy.host("secret.example.com"),
            HostRule::Denied("secret.example.com".into())
        );
        assert_eq!(policy.host("crates.io"), HostRule::NotAllowed);
        let open = WebPolicy {
            deny: domains(&["*"]),
            ..WebPolicy::default()
        };
        assert!(matches!(open.host("docs.rs"), HostRule::Denied(_)));
    }

    #[test]
    fn only_hosts_named_exactly_may_be_private() {
        let policy = WebPolicy {
            allow: domains(&["127.0.0.1", "*.corp", "[::1]"]),
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
}
