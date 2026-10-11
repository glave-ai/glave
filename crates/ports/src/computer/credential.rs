//! A credential for a workspace's sources: what git asks a credential helper for when it
//! fetches or pushes over HTTPS. It crosses the wire to a computer only inside `workspace/open`
//! and lives there in memory, never on a disk.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::types::ComputerError;

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
