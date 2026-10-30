//! Git hosts (`specs/session.md`, "Git hosts"): the person on this computer connects their
//! GitHub, or any git host with a token, and git and the host's own tool sign in with it from
//! then on, asking for it at each use. The host keeps them sealed and hands them out; the
//! server decides who may ask.

use glave_event::{CredentialId, Secret};
use glave_ports::{BlobStore, BoxFuture, Caller, EventLog, ModelProvider};

use crate::protocol::{ApiError, CredentialInfo, DeviceCode, Reply};
use crate::providers::theirs;
use crate::server::Server;
use crate::words::Sentence;

/// A git host to connect, as `credential/connect` asks.
#[derive(Debug, Clone)]
pub struct NewCredential {
    pub url: String,
    pub token: Secret,
    pub username: Option<String>,
}

/// What keeps a person's git hosts: the host, which also hands their credentials to commands.
pub trait GitHosts: Send + Sync {
    /// Checks `new` with its host where it can, keeps it, and answers with it.
    fn connect(&self, new: NewCredential) -> BoxFuture<'_, Result<CredentialInfo, ApiError>>;

    /// The hosts kept.
    fn list(&self) -> BoxFuture<'_, Result<Vec<CredentialInfo>, ApiError>>;

    /// Removes `credential`, destroying its token.
    fn remove(&self, credential: CredentialId) -> BoxFuture<'_, Result<(), ApiError>>;

    /// The page where the person chooses which repositories GitHub's app may use, if it has
    /// one.
    fn install(&self) -> Option<String>;

    /// Where signing in to GitHub in the browser goes: github.com, or the GitHub Enterprise
    /// Server the organization's own app is on.
    fn github(&self) -> String;

    /// Starts signing in to GitHub in the browser.
    fn start_device(&self) -> BoxFuture<'_, Result<DeviceCode, ApiError>>;

    /// Waits a little for the sign-in `flow`: the host once connected, none while it waits.
    fn poll_device(&self, flow: String) -> BoxFuture<'_, Result<Option<CredentialInfo>, ApiError>>;
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The host's git hosts, for the person on this computer only.
    fn git_hosts_for(&self, caller: &Caller) -> Result<&dyn GitHosts, ApiError> {
        let hosts = self.inner.git_hosts.as_deref();
        let elsewhere = Sentence::AccountsOnItsComputer;
        theirs(caller, hosts, Sentence::AccountsNotHere, elsewhere)
    }

    /// Answers `credential/connect`.
    pub(crate) async fn connect_credential(
        &self,
        caller: &Caller,
        new: NewCredential,
    ) -> Result<Reply, ApiError> {
        let credential = self.git_hosts_for(caller)?.connect(new).await?;
        Ok(Reply::Credential { credential })
    }

    /// Answers `credential/list`.
    pub(crate) async fn list_credentials(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let hosts = self.git_hosts_for(caller)?;
        Ok(Reply::Credentials {
            credentials: hosts.list().await?,
            install: hosts.install(),
            github: hosts.github(),
        })
    }

    /// Answers `credential/remove`.
    pub(crate) async fn remove_credential(
        &self,
        caller: &Caller,
        credential: CredentialId,
    ) -> Result<Reply, ApiError> {
        self.git_hosts_for(caller)?.remove(credential).await?;
        Ok(Reply::Done {})
    }

    /// Answers `credential/device/start`.
    pub(crate) async fn start_device(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let device = self.git_hosts_for(caller)?.start_device().await?;
        Ok(Reply::Device { device })
    }

    /// Answers `credential/device/poll`.
    pub(crate) async fn poll_device(
        &self,
        caller: &Caller,
        flow: String,
    ) -> Result<Reply, ApiError> {
        let hosts = self.git_hosts_for(caller)?;
        Ok(match hosts.poll_device(flow.clone()).await? {
            Some(credential) => Reply::Credential { credential },
            None => Reply::Waiting { flow },
        })
    }
}
