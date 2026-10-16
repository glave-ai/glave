//! Model accounts (`specs/session.md`, "Model accounts"): the person on this computer connects
//! their own key for a service, and the server's models follow at once. The host keeps the
//! accounts and builds models from them; the server decides who may ask, and offers the
//! models each change leaves.

use glave_event::{AuthMethod, ProviderId, ProviderKind, Secret};
use glave_harness::Caller;
use glave_ports::{BlobStore, BoxFuture, EventLog, ModelProvider};

use crate::protocol::{ApiError, ModelInfo, ProviderInfo, Reply, code};
use crate::server::Server;

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

/// What keeps model accounts for a server: the host, which builds its models from them. Each
/// change answers with the models the server runs from then on.
pub trait Providers: Send + Sync {
    /// Checks `new` with its service, keeps it, and answers with it and the models now.
    fn connect(&self, new: NewProvider) -> BoxFuture<'_, Result<(ProviderInfo, Models), ApiError>>;

    /// The accounts kept.
    fn list(&self) -> BoxFuture<'_, Result<Vec<ProviderInfo>, ApiError>>;

    /// Removes `provider`, destroying its key, and answers with the models now.
    fn remove(&self, provider: ProviderId) -> BoxFuture<'_, Result<Models, ApiError>>;
}

/// Whether `caller` is the person on this computer: over a local transport, or signed in with
/// the local token from a browser here.
pub(crate) fn local(caller: &Caller) -> bool {
    caller
        .auth
        .as_ref()
        .is_none_or(|auth| auth.method == AuthMethod::Local)
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
        let Some(providers) = self.inner.providers.as_deref() else {
            return Err(ApiError::new(
                code::NOT_AVAILABLE,
                "this server's models are set up by whoever runs it, not from here",
            ));
        };
        if !local(caller) {
            return Err(ApiError::new(
                code::NOT_ALLOWED,
                "model accounts are connected on the computer glave runs on, by its person",
            ));
        }
        Ok(providers)
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

    /// Runs `models` from now on: threads that chose none go to its default from their next
    /// turn, and `model/list` names them.
    pub(crate) fn offer(&self, models: Models) {
        let default = models.default.unwrap_or_default();
        let catalog = crate::models::catalog(&default.0, models.offered);
        self.inner.harness.set_default_model(default.0, default.1);
        self.set_catalog(catalog);
    }
}
