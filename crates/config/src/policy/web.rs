//! `[web]` in `config.toml`: whether `web_fetch` is on, which hosts it may reach, and whether
//! it may reach private addresses. The table is one choice: the nearest file that has one wins.

use glave_ports::settings::{Domain, WebPolicy};
use serde::Deserialize;

/// `[web]` as written. Every key is optional.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebFile {
    /// Whether `web_fetch` is offered; on when unset.
    pub fetch: Option<bool>,
    /// Hosts it may fetch (`docs.rs`, `*.example.com`); every host when empty.
    #[serde(default)]
    pub allow: Vec<String>,
    /// Hosts it may never fetch; a deny wins over an allow.
    #[serde(default)]
    pub deny: Vec<String>,
    /// Whether any host may lead to a private address; off when unset. A host `allow` names
    /// exactly may whatever this says.
    pub private_addresses: Option<bool>,
}

impl WebFile {
    /// The policy `web_fetch` runs under; an error names a pattern that is not one.
    pub fn policy(&self) -> crate::Result<WebPolicy> {
        let domains = |key: &str, patterns: &[String]| {
            patterns
                .iter()
                .map(|pattern| {
                    Domain::parse(pattern)
                        .map_err(|error| crate::Error::within(format!("[web] {key}"), error))
                })
                .collect::<crate::Result<Vec<_>>>()
        };
        Ok(WebPolicy {
            fetch: self.fetch.unwrap_or(true),
            allow: domains("allow", &self.allow)?,
            deny: domains("deny", &self.deny)?,
            private_addresses: self.private_addresses.unwrap_or(false),
        })
    }

    /// What a project's `[web]` sets that would let `web_fetch` reach further than glave's
    /// default: private addresses, or hosts named in `allow`, which may then be private. These
    /// need the person's trust in the project.
    pub(crate) fn loosening(&self) -> Option<String> {
        let mut parts = Vec::new();
        if self.private_addresses == Some(true) {
            parts.push("[web] private_addresses = true".to_owned());
        }
        if !self.allow.is_empty() {
            parts.push(format!("[web] allow = {:?}", self.allow));
        }
        (!parts.is_empty()).then(|| parts.join(", "))
    }

    /// This `[web]` without what needs trust.
    #[must_use]
    pub(crate) fn without_trust(&self) -> Self {
        Self {
            allow: Vec::new(),
            private_addresses: None,
            ..self.clone()
        }
    }

    /// The table in a few words, for `glave policy explain`.
    pub fn describe(&self) -> String {
        if self.fetch == Some(false) {
            return "fetch off".to_owned();
        }
        let mut parts = vec!["fetch on".to_owned()];
        if !self.allow.is_empty() {
            parts.push(format!("allow {:?}", self.allow));
        }
        if !self.deny.is_empty() {
            parts.push(format!("deny {:?}", self.deny));
        }
        if self.private_addresses == Some(true) {
            parts.push("private addresses".to_owned());
        }
        parts.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> WebFile {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn the_defaults_fetch_public_hosts_only() {
        let policy = WebFile::default().policy().unwrap();
        assert_eq!(policy, WebPolicy::default());
        assert_eq!(WebFile::default().describe(), "fetch on");
        assert_eq!(WebFile::default().loosening(), None);
    }

    #[test]
    fn a_table_is_read_and_checked() {
        let web = read("fetch = true\nallow = [\"docs.rs\", \"*.corp\"]\ndeny = [\"x.corp\"]\n");
        let policy = web.policy().unwrap();
        assert_eq!(policy.allow.len(), 2);
        assert!(web.describe().contains("deny [\"x.corp\"]"));
        let bad = read("deny = [\"https://x.com/\"]").policy().unwrap_err();
        assert!(format!("{bad:#}").contains("[web] deny"), "{bad:#}");
        assert!(toml::from_str::<WebFile>("search = true").is_err());
        assert_eq!(read("fetch = false").describe(), "fetch off");
        assert!(!read("fetch = false").policy().unwrap().fetch);
    }

    #[test]
    fn reaching_further_needs_trust_and_is_dropped_without_it() {
        let web = read("private_addresses = true\nallow = [\"10.0.0.5\"]\ndeny = [\"a.b\"]");
        let loosening = web.loosening().unwrap();
        assert!(loosening.contains("private_addresses") && loosening.contains("10.0.0.5"));
        let kept = web.without_trust();
        assert_eq!(kept.loosening(), None);
        assert_eq!(kept.deny, ["a.b"]);
    }
}
