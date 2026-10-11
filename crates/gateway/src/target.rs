//! A target: one model at one provider that a route may send to, with what it can do and,
//! when known, what it costs.

use std::fmt;
use std::sync::Arc;

use glave_ports::{BoxFuture, Completion, Delta, ModelProvider, ModelRequest, ProviderError};

use crate::price::Price;

/// A model provider behind a target. Every [`ModelProvider`] is one; this form can be shared
/// between targets of different provider types.
pub trait Model: Send + Sync {
    fn send<'a>(
        &'a self,
        request: &'a ModelRequest,
        on_delta: &'a mut (dyn FnMut(Delta) + Send),
    ) -> BoxFuture<'a, Result<Completion, ProviderError>>;
}

impl<P: ModelProvider> Model for P {
    fn send<'a>(
        &'a self,
        request: &'a ModelRequest,
        on_delta: &'a mut (dyn FnMut(Delta) + Send),
    ) -> BoxFuture<'a, Result<Completion, ProviderError>> {
        Box::pin(self.complete(request, on_delta))
    }
}

/// What a target can do. Unknown limits are `None`, and nothing is skipped for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// Whether it can call tools.
    pub tools: bool,
    /// Whether it accepts its own opaque reasoning back.
    pub reasoning: bool,
    /// Whether it reads images.
    pub images: bool,
    /// Its context window, in tokens.
    pub context_tokens: Option<u64>,
    /// The longest answer it gives when a request sets no limit, in tokens: what a budget
    /// reserves for the answer.
    pub max_output_tokens: Option<u32>,
}

impl Default for Capabilities {
    /// Everything, with no known limits: what a model is assumed to do until configured.
    fn default() -> Self {
        Self {
            tools: true,
            reasoning: true,
            images: true,
            context_tokens: None,
            max_output_tokens: None,
        }
    }
}

/// One model at one provider.
#[derive(Clone)]
pub struct Target {
    /// The target's name, recorded with every answer it serves: `provider/model` unless set.
    pub name: String,
    /// The provider's short name, as its answers and opaque reasoning record it, such as
    /// `openrouter`.
    pub provider: String,
    /// The model name sent to the provider.
    pub model: String,
    pub can: Capabilities,
    /// What it costs per token, when configured.
    pub price: Option<Price>,
    pub(crate) handle: Arc<dyn Model>,
}

impl Target {
    /// `model` at `provider`, reached through `handle`, assumed able to do everything.
    pub fn new(
        handle: Arc<dyn Model>,
        provider: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        let provider = provider.into();
        let model = model.into();
        Self {
            name: format!("{provider}/{model}"),
            provider,
            model,
            can: Capabilities::default(),
            price: None,
            handle,
        }
    }

    #[must_use]
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Another model at the same provider, through the same handle: assumed able to do
    /// everything, with no price, and named `provider/model`.
    #[must_use]
    pub fn sibling(&self, model: impl Into<String>) -> Self {
        Self::new(Arc::clone(&self.handle), self.provider.clone(), model)
    }

    #[must_use]
    pub fn with_capabilities(mut self, can: Capabilities) -> Self {
        self.can = can;
        self
    }

    #[must_use]
    pub fn with_price(mut self, price: Price) -> Self {
        self.price = Some(price);
        self
    }
}

impl fmt::Debug for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Target")
            .field("name", &self.name)
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("can", &self.can)
            .field("price", &self.price)
            .finish_non_exhaustive()
    }
}
