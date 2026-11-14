//! Each person's git hosts, apart (`specs/session.md`, "Git hosts"): where people have
//! computers of their own, everyone connects their own, kept in their own accounts ledger, in
//! their personal project in the organization they work in, and their commands ask those alone.
//! The person on this computer's are the ones this computer's commands ask; anyone else's are
//! opened when first asked for and kept while asked for often.

use std::sync::{Arc, Mutex, PoisonError};

use glave_config::policy::GithubFile;
use glave_event::PrincipalId;
use glave_ports::computer::{CredentialAsk, CredentialHost, GitCredential, GitCredentials};
use glave_ports::{BoxFuture, OrgId};
use glave_session::protocol::ApiError;
use glave_session::{GitHosts, PeoplesGitHosts};

use super::Hosts;
use crate::data::Data;

/// The people whose git hosts are kept open at once, beside the person on this computer's: the
/// one asked for least recently goes first.
const KEPT_OPEN: usize = 256;

/// A person, by their organization and id.
type Whose = (OrgId, PrincipalId);

/// Each person's git hosts, by their organization and id.
pub(crate) struct PeoplesHosts {
    kept: Arc<Kept>,
}

/// What [`PeoplesHosts`] and the credentials it hands out share.
struct Kept {
    store: Data,
    github: Option<GithubFile>,
    /// The person on this computer, in the server's own organization, and their git hosts.
    own: (Whose, Arc<Hosts>),
    /// Anyone else's opened, the most recently asked for last.
    others: Mutex<Vec<(Whose, Arc<Hosts>)>>,
}

impl PeoplesHosts {
    /// Everyone's git hosts in `store`, `own` those of `person` in `org`, signing in to GitHub
    /// through `github`'s app when set, else through glave's own.
    pub(crate) fn new(
        store: Data,
        github: Option<GithubFile>,
        (org, person): Whose,
        own: Arc<Hosts>,
    ) -> Self {
        Self {
            kept: Arc::new(Kept {
                store,
                github,
                own: ((org, person), own),
                others: Mutex::default(),
            }),
        }
    }
}

impl Kept {
    /// The git hosts of `person` in `org`, opened the first time.
    async fn of(&self, org: OrgId, person: PrincipalId) -> anyhow::Result<Arc<Hosts>> {
        let whose = (org, person);
        if whose == self.own.0 {
            return Ok(Arc::clone(&self.own.1));
        }
        if let Some(hosts) = self.recent(whose, None) {
            return Ok(hosts);
        }
        let ledger = crate::accounts::open_in(&self.store, org, person).await?;
        let opened = Hosts::open(ledger, self.github.as_ref()).await?;
        // Two asking at once keep the first opened, so a sign-in waiting in one is never lost.
        self.recent(whose, Some(opened))
            .ok_or_else(|| anyhow::anyhow!("could not keep the git hosts opened"))
    }

    /// The git hosts kept open for `whose`, now the most recently asked for, keeping `opened`
    /// for them when none are.
    fn recent(&self, whose: Whose, opened: Option<Arc<Hosts>>) -> Option<Arc<Hosts>> {
        let mut others = self.others.lock().unwrap_or_else(PoisonError::into_inner);
        let found = match others.iter().position(|(kept, _)| *kept == whose) {
            Some(at) => others.remove(at).1,
            None => opened?,
        };
        others.push((whose, Arc::clone(&found)));
        let over = others.len().saturating_sub(KEPT_OPEN);
        others.drain(..over);
        Some(found)
    }
}

impl PeoplesGitHosts for PeoplesHosts {
    fn of(
        &self,
        org: OrgId,
        person: PrincipalId,
    ) -> BoxFuture<'_, Result<Arc<dyn GitHosts>, ApiError>> {
        Box::pin(async move {
            let hosts = self
                .kept
                .of(org, person)
                .await
                .map_err(|e| ApiError::internal(&e))?;
            Ok(hosts as Arc<dyn GitHosts>)
        })
    }

    fn credentials(&self, org: OrgId, person: PrincipalId) -> Arc<dyn GitCredentials> {
        Arc::new(Theirs {
            kept: Arc::clone(&self.kept),
            org,
            person,
        })
    }
}

/// The credentials of one person's commands: their git hosts, read when asked.
struct Theirs {
    kept: Arc<Kept>,
    org: OrgId,
    person: PrincipalId,
}

impl Theirs {
    /// Their git hosts, or none when they can't be read, which commands then go without.
    async fn hosts_now(&self) -> Option<Arc<Hosts>> {
        match self.kept.of(self.org, self.person).await {
            Ok(hosts) => Some(hosts),
            Err(error) => {
                tracing::warn!("could not read a person's git hosts for a command: {error:#}");
                None
            }
        }
    }
}

impl GitCredentials for Theirs {
    fn hosts(&self) -> BoxFuture<'_, Vec<CredentialHost>> {
        Box::pin(async move {
            match self.hosts_now().await {
                Some(hosts) => hosts.hosts().await,
                None => Vec::new(),
            }
        })
    }

    fn request<'a>(&'a self, ask: &'a CredentialAsk) -> BoxFuture<'a, Option<GitCredential>> {
        Box::pin(async move { self.hosts_now().await?.request(ask).await })
    }
}

#[cfg(test)]
mod tests;
