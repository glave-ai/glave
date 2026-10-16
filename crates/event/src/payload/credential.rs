//! Credentials for git hosts: which forge a host runs, so git and that forge's own
//! command-line tool reach it with the person's token, handed over for each use
//! (`specs/computers.md`, rule 8).

use serde::{Deserialize, Serialize};

/// Which forge a git host runs, as far as glave could tell: what its API and its
/// command-line tool are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Forge {
    /// GitHub, or GitHub Enterprise Server: `gh`.
    Github,
    /// GitLab, its own or self-managed: `glab`.
    Gitlab,
    /// Gitea or Forgejo: `tea`.
    Gitea,
    /// Bitbucket Cloud.
    Bitbucket,
    /// Any other host git reaches over HTTPS.
    Other,
}
