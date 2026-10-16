//! Credentials on a computer (`specs/computers.md`, rule 8): what git asks a credential helper
//! for when it fetches or pushes over HTTPS. A workspace's sources may bring one inside
//! `workspace/open`, kept in memory; a command's git and the forge's own command-line tool ask
//! [`Credentials`] for one each time they need it (`credentials/request`), so the value
//! reaches only the process that asked, never a disk or the command's environment.

use std::fmt;

use glave_event::Forge;
use serde::{Deserialize, Serialize};

use super::types::ComputerError;
use crate::tools::BoxFuture;

/// A host a command's git may ask a credential for, and which forge it runs, so that
/// forge's own command-line tool (`gh`, `glab`, `tea`) runs signed in to it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialHost {
    /// Where the host is, as git's `credential.<url>` settings name it: `https://github.com`.
    pub url: String,
    pub forge: Forge,
}

/// A forge's own command-line tool, which runs signed in to one connected host of its forge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForgeTool {
    Gh,
    Glab,
    Tea,
}

impl ForgeTool {
    /// The tool's command.
    pub fn name(self) -> &'static str {
        match self {
            Self::Gh => "gh",
            Self::Glab => "glab",
            Self::Tea => "tea",
        }
    }

    fn of(forge: Forge) -> Option<Self> {
        match forge {
            Forge::Github => Some(Self::Gh),
            Forge::Gitlab => Some(Self::Glab),
            Forge::Gitea => Some(Self::Tea),
            Forge::Bitbucket | Forge::Other => None,
        }
    }

    /// The host the tool talks to by default, when it has one.
    pub fn home(self) -> Option<&'static str> {
        match self {
            Self::Gh => Some("https://github.com"),
            Self::Glab => Some("https://gitlab.com"),
            Self::Tea => None,
        }
    }

    /// Each tool with the host it runs signed in to among `hosts`: its default host when that
    /// is there, else the first of its forge. A tool takes one token, so one host each.
    pub fn signed_in(hosts: &[CredentialHost]) -> Vec<(Self, &CredentialHost)> {
        let mut chosen = Vec::new();
        for tool in [Self::Gh, Self::Glab, Self::Tea] {
            let mine = || {
                hosts
                    .iter()
                    .filter(|host| Self::of(host.forge) == Some(tool))
            };
            let home = mine().find(|host| Some(host.url.as_str()) == tool.home());
            if let Some(host) = home.or_else(|| mine().next()) {
                chosen.push((tool, host));
            }
        }
        chosen
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

/// A user name and password (or token) for a repository over HTTPS. Its debug output hides
/// the password, so it never reaches a log by accident.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitCredential {
    pub username: String,
    pub password: String,
}

impl GitCredential {
    /// A credential, refused when either part could not be handed to git as one line.
    pub fn new(
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<Self, ComputerError> {
        let credential = Self {
            username: username.into(),
            password: password.into(),
        };
        credential.check()?;
        Ok(credential)
    }

    /// Refuses a credential git's helper protocol could not carry: empty, or with a line
    /// break or a NUL in it.
    pub fn check(&self) -> Result<(), ComputerError> {
        let bad = |text: &str| text.is_empty() || text.contains(['\n', '\r', '\0']);
        if bad(&self.username) || bad(&self.password) {
            return Err(ComputerError::Refused(
                "a git credential needs a user name and a password, each one line".into(),
            ));
        }
        Ok(())
    }
}

impl fmt::Debug for GitCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitCredential")
            .field("username", &self.username)
            .field("password", &"<hidden>")
            .finish()
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
}
