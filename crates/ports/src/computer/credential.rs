//! Credentials on a computer (`specs/computers.md`, rule 8): what git asks a credential helper
//! for when it fetches or pushes over HTTPS. A workspace's sources may bring one inside
//! `workspace/open`, kept in memory; a command's git and the forge's own command-line tool ask
//! [`Credentials`] for one each time they need it (`credentials/request`), so the value
//! reaches only the process that asked, never a disk or the command's environment.

use glave_event::{Forge, Secret};
use serde::{Deserialize, Serialize};

use super::types::ComputerError;
use crate::tools::BoxFuture;

/// A host a command's git may ask a credential for, and which forge it runs, so that forge's
/// own command-line tool may run signed in to it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialHost {
    /// Where the host is, as git's `credential.<url>` settings name it: `https://github.com`.
    /// Only an [`origin`](Self::origin) is ever offered to a command.
    pub url: String,
    pub forge: Forge,
}

impl CredentialHost {
    /// The host at `url`, given as its origin in any case, with or without a final `/`, kept
    /// as glave offers it: in lower case. None when it is not an [`origin`](Self::origin).
    pub fn named(url: &str, forge: Forge) -> Option<Self> {
        let url = url.trim().trim_end_matches('/').to_ascii_lowercase();
        let host = Self { url, forge };
        host.origin().is_some().then_some(host)
    }

    /// Its scheme and its host with any port, when `url` is an origin glave hands a credential
    /// for: `https://` and a host of letters, digits, `.` and `-` with any port, or `http://` to
    /// this computer, and nothing more: no path, user name, query or space.
    pub fn origin(&self) -> Option<(&str, &str)> {
        let (scheme, host) = self.url.split_once("://")?;
        let (name, port) = match host.rsplit_once(':') {
            Some((name, port)) if !name.ends_with(':') => (name, Some(port)),
            _ => (host, None),
        };
        let named = name == "[::1]"
            || (!name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'));
        let ported = port.is_none_or(|port| {
            !port.is_empty() && port.len() <= 5 && port.chars().all(|c| c.is_ascii_digit())
        });
        let local = matches!(name, "localhost" | "127.0.0.1" | "[::1]");
        let allowed = scheme == "https" || (scheme == "http" && local);
        (named && ported && allowed).then_some((scheme, host))
    }

    /// Whether `ask` is for this host: its scheme exactly, its host and port in any case.
    pub fn is_for(&self, ask: &CredentialAsk) -> bool {
        self.origin().is_some_and(|(scheme, host)| {
            ask.protocol == scheme && ask.host.eq_ignore_ascii_case(host)
        })
    }
}

/// A forge's own command-line tool, run signed in to one connected host. Only tools that send
/// their token to that host alone: `gh` keeps `GH_TOKEN` for github.com, and `tea` for the
/// instance it is given. `glab` sends `GITLAB_TOKEN` to any host it is pointed at, and `gh`
/// sends GitHub Enterprise's token to any host but github.com, so neither runs signed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForgeTool {
    Gh,
    Tea,
}

impl ForgeTool {
    /// The tool's command.
    pub fn name(self) -> &'static str {
        match self {
            Self::Gh => "gh",
            Self::Tea => "tea",
        }
    }

    /// Each tool with the host it runs signed in to among `hosts`: `gh` for github.com, `tea`
    /// for the first Gitea or Forgejo host. A tool takes one token, so one host each.
    pub fn signed_in(hosts: &[CredentialHost]) -> Vec<(Self, &CredentialHost)> {
        let usable = || hosts.iter().filter(|host| host.origin().is_some());
        let github = usable()
            .find(|host| host.forge == Forge::Github && host.url == "https://github.com")
            .map(|host| (Self::Gh, host));
        let gitea = usable()
            .find(|host| host.forge == Forge::Gitea)
            .map(|host| (Self::Tea, host));
        github.into_iter().chain(gitea).collect()
    }
}

/// What a command's credential helper asked for: where git, or a forge's tool, is going.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialAsk {
    /// `https`, as git says it.
    pub protocol: String,
    /// The host and any port: `github.com`, `git.example.com:8443`.
    pub host: String,
    /// The repository's path, when git sends it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Where a command's credentials come from, asked for at each use: the hosts a person
/// connected, sealed in the store, or an organization's grants. Either may refuse.
pub trait Credentials: Send + Sync {
    /// The hosts it may answer for now.
    fn hosts(&self) -> BoxFuture<'_, Vec<CredentialHost>>;

    /// The credential for `ask`, or none: an unknown host, or one it will not hand out.
    fn request<'a>(&'a self, ask: &'a CredentialAsk) -> BoxFuture<'a, Option<GitCredential>>;
}

/// A user name and password (or token) for a repository over HTTPS. The password never shows
/// in `Debug`, so it never reaches a log by accident.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitCredential {
    pub username: String,
    #[serde(serialize_with = "glave_event::serialize_exposed")]
    pub password: Secret,
}

impl GitCredential {
    /// A credential, refused when either part could not be handed to git as one line.
    pub fn new(
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<Self, ComputerError> {
        let credential = Self {
            username: username.into(),
            password: Secret::new(password),
        };
        credential.check()?;
        Ok(credential)
    }

    /// Refuses a credential git's helper protocol could not carry: empty, or with a line
    /// break or a NUL in it.
    pub fn check(&self) -> Result<(), ComputerError> {
        let bad = |text: &str| text.is_empty() || text.contains(['\n', '\r', '\0']);
        if bad(&self.username) || bad(self.password.expose()) {
            return Err(ComputerError::Refused(
                "a git credential needs a user name and a password, each one line".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_password_never_shows_and_bad_lines_are_refused() {
        let credential = GitCredential::new("x-access-token", "ghp_secret").unwrap();
        assert!(!format!("{credential:?}").contains("ghp_secret"));
        assert!(GitCredential::new("a", "b\nc").is_err());
        assert!(GitCredential::new("", "b").is_err());
    }

    fn host(url: &str, forge: Forge) -> CredentialHost {
        CredentialHost {
            url: url.into(),
            forge,
        }
    }

    #[test]
    fn only_a_plain_origin_is_offered() {
        let ask = |protocol: &str, host: &str| CredentialAsk {
            protocol: protocol.into(),
            host: host.into(),
            path: None,
        };
        let github = host("https://github.com", Forge::Github);
        assert!(github.is_for(&ask("https", "GitHub.com")));
        assert!(!github.is_for(&ask("http", "github.com")));
        assert!(!github.is_for(&ask("https", "github.com.evil.example")));
        for offered in [
            "https://git.example.com:8443",
            "http://127.0.0.1:3000",
            "http://[::1]:3000",
            "http://localhost",
        ] {
            assert!(host(offered, Forge::Other).origin().is_some(), "{offered}");
        }
        for refused in [
            "http://github.com",
            "http://localhost.evil.example",
            "https://",
            "github.com",
            "https://me@github.com",
            "https://github.com/acme",
            "https://x\ntouch pwned",
            "https://git hub.com",
            "https://github.com:99999x",
            "https://github.com?x",
            "ftp://github.com",
        ] {
            assert_eq!(host(refused, Forge::Other).origin(), None, "{refused}");
            assert_eq!(
                CredentialHost::named(refused, Forge::Other),
                None,
                "{refused}"
            );
        }
        assert_eq!(
            CredentialHost::named(" HTTPS://GHE.Example.com:8443/ ", Forge::Github),
            Some(host("https://ghe.example.com:8443", Forge::Github))
        );
    }

    #[test]
    fn only_tools_that_keep_their_token_to_one_host_run_signed_in() {
        let hosts = [
            host("https://git.example.com", Forge::Github),
            host("https://gitlab.com", Forge::Gitlab),
            host("https://code.example.com", Forge::Gitea),
            host("https://github.com", Forge::Github),
        ];
        let chosen: Vec<(&str, &str)> = ForgeTool::signed_in(&hosts)
            .into_iter()
            .map(|(tool, host)| (tool.name(), host.url.as_str()))
            .collect();
        assert_eq!(
            chosen,
            [
                ("gh", "https://github.com"),
                ("tea", "https://code.example.com")
            ]
        );
    }
}
