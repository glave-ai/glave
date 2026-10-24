//! Model accounts (`specs/session.md`, "Model accounts"): the person on this computer connects
//! their own key for a service, and the server's models follow at once. The host keeps the
//! accounts and builds models from them; the server decides who may ask, and offers the
//! models each change leaves.

use glave_event::{AuthMethod, ProviderId, ProviderKind, Secret};
use glave_ports::{BlobStore, BoxFuture, Caller, EventLog, ModelProvider};

use crate::protocol::{ApiError, ModelInfo, ProviderInfo, Reply, code};
use crate::server::Server;
use crate::words::Sentence;

/// A model account to connect, as `provider/connect` asks.
#[derive(Debug, Clone)]
pub struct NewProvider {
    pub kind: ProviderKind,
    pub key: Secret,
    pub url: Option<String>,
}

/// The models a server runs once accounts change: the default and its provider's short name,
/// then every model a thread may choose. No default while nothing is set up.
#[derive(Debug, Clone, Default)]
pub struct Models {
    pub default: Option<(String, String)>,
    pub offered: Vec<ModelInfo>,
}

/// How a refresh of the accounts' catalogs went.
#[derive(Debug, Default)]
pub struct Refreshed {
    /// Whether any account's models changed: the server runs them again.
    pub changed: bool,
    /// Why a service's models could not be listed: its catalog stays as it was.
    pub failed: Option<ApiError>,
}

/// What keeps model accounts for a server: the host, which builds its models from them. Each
/// change answers with the models the server runs from then on.
pub trait Providers: Send + Sync {
    /// Checks `new` with its service, keeps it, and answers with it and the models now.
    fn connect(&self, new: NewProvider) -> BoxFuture<'_, Result<(ProviderInfo, Models), ApiError>>;

    /// The accounts kept.
    fn list(&self) -> BoxFuture<'_, Result<Vec<ProviderInfo>, ApiError>>;

    /// Removes `provider`, destroying its key, and answers with the models now.
    fn remove(&self, provider: ProviderId) -> BoxFuture<'_, Result<Models, ApiError>>;

    /// Whether the accounts' catalogs are due their daily refresh. True once for each, so
    /// the caller refreshes them.
    fn due(&self) -> bool;

    /// Lists every account's models again with its service and keeps what it says: only the
    /// catalogs a day old or more, or with `all`, every one.
    fn refresh(&self, all: bool) -> BoxFuture<'_, Result<Refreshed, ApiError>>;

    /// Builds the models of the accounts and catalogs kept now, and answers with them.
    fn models(&self) -> BoxFuture<'_, Result<Models, ApiError>>;
}

/// Whether `caller` is the person on this computer: over a local transport, or signed in with
/// the local token from a browser here.
pub(crate) fn local(caller: &Caller) -> bool {
    caller
        .auth
        .as_ref()
        .is_none_or(|auth| auth.method == AuthMethod::Local)
}

/// What the host keeps for the person on this computer, `kept`, for `caller`: refused with
/// `missing` where the host keeps none, and with `elsewhere` for anyone but that person.
pub(crate) fn theirs<'a, T: ?Sized>(
    caller: &Caller,
    kept: Option<&'a T>,
    missing: Sentence,
    elsewhere: Sentence,
) -> Result<&'a T, ApiError> {
    let kept = kept.ok_or_else(|| ApiError::new(code::NOT_AVAILABLE, missing))?;
    if !local(caller) {
        return Err(ApiError::new(code::NOT_ALLOWED, elsewhere));
    }
    Ok(kept)
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The host's accounts, for the person on this computer only: elsewhere an organization
    /// sets up its models itself.
    fn providers_for(&self, caller: &Caller) -> Result<&dyn Providers, ApiError> {
        let providers = self.inner.providers.as_deref();
        let elsewhere = Sentence::ModelAccountsOnItsComputer;
        theirs(caller, providers, Sentence::ModelsNotHere, elsewhere)
    }

    /// Answers `provider/connect`.
    pub(crate) async fn connect_provider(
        &self,
        caller: &Caller,
        new: NewProvider,
    ) -> Result<Reply, ApiError> {
        let providers = self.providers_for(caller)?;
        // One change at a time, so the models offered are those of the last.
        let _changing = self.inner.changing_models.lock().await;
        let (provider, models) = providers.connect(new).await?;
        self.offer(models);
        Ok(Reply::Provider { provider })
    }

    /// Answers `provider/list`.
    pub(crate) async fn list_providers(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let providers = self.providers_for(caller)?.list().await?;
        Ok(Reply::Providers { providers })
    }

    /// Answers `provider/remove`.
    pub(crate) async fn remove_provider(
        &self,
        caller: &Caller,
        provider: ProviderId,
    ) -> Result<Reply, ApiError> {
        let providers = self.providers_for(caller)?;
        let _changing = self.inner.changing_models.lock().await;
        let models = providers.remove(provider).await?;
        self.offer(models);
        Ok(Reply::Done {})
    }

    /// Answers `provider/refresh`: every account's models listed again now.
    pub(crate) async fn refresh_providers(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let providers = self.providers_for(caller)?;
        let refreshed = providers.refresh(true).await?;
        self.rerun(providers, refreshed.changed).await?;
        if let Some(failed) = refreshed.failed {
            return Err(failed);
        }
        self.list_providers(caller).await
    }

    /// Refreshes the accounts' catalogs, when they are due, in a task the server owns: no one
    /// waits for it, and the models follow once it ends.
    pub(crate) fn refresh_when_due(&self) {
        let Some(providers) = self.inner.providers.clone() else {
            return;
        };
        if !providers.due() {
            return;
        }
        let server = self.clone();
        let task = tokio::spawn(async move {
            let refreshed = match providers.refresh(false).await {
                Ok(refreshed) => refreshed,
                Err(error) => Refreshed {
                    changed: false,
                    failed: Some(error),
                },
            };
            if let Some(failed) = &refreshed.failed {
                tracing::warn!("could not refresh the models' catalogs: {}", failed.message);
            }
            if let Err(error) = server.rerun(providers.as_ref(), refreshed.changed).await {
                tracing::warn!("could not run the refreshed models: {}", error.message);
            }
        });
        self.own(task);
    }

    /// Runs the models of the accounts kept now, when `changed`.
    async fn rerun(&self, providers: &dyn Providers, changed: bool) -> Result<(), ApiError> {
        if changed {
            let _changing = self.inner.changing_models.lock().await;
            self.offer(providers.models().await?);
        }
        Ok(())
    }

    /// Runs `models` from now on: threads that chose none go to its default from their next
    /// turn, and `model/list` names them.
    pub(crate) fn offer(&self, models: Models) {
        let default = models.default.unwrap_or_default();
        let catalog = crate::models::catalog(&default.0, models.offered);
        self.inner.harness.set_default_model(default.0, default.1);
        self.set_catalog(catalog);
    }
}
