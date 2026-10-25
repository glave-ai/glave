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

#[deny(clippy::missing_trait_methods)]
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
    /// The service it is at, in a person's words, such as `OpenRouter`, for receipts.
    pub service: Option<String>,
    /// The model it serves, by a name it shares with targets at other providers that serve the
    /// same one: a thread that chooses any of them is sent to them all, tried in order.
    pub same_model: Option<String>,
    /// The reasoning levels it offers, in its provider's names.
    efforts: Vec<String>,
    /// The one of `efforts` its model uses when asked for none, as its service's catalog says.
    default_effort: Option<String>,
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
            service: None,
            same_model: None,
            efforts: Vec::new(),
            default_effort: None,
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

    /// The target, offering the reasoning levels `efforts`, its model using `default` when
    /// asked for none; a default it does not offer is not known.
    #[must_use]
    pub fn with_efforts(mut self, efforts: Vec<String>, default: Option<String>) -> Self {
        self.default_effort = default.filter(|default| efforts.contains(default));
        self.efforts = efforts;
        self
    }

    /// The reasoning level a request asking for `asked` is sent to this target with: that
    /// level when the target offers it, else the one its model uses unasked, sent so the
    /// model reasons alike at every service; none when neither is known, which leaves it to
    /// the model's maker.
    pub fn effort_for<'a>(&'a self, asked: Option<&'a str>) -> Option<&'a str> {
        asked
            .filter(|asked| self.efforts.iter().any(|offered| offered == asked))
            .or(self.default_effort.as_deref())
    }

    /// The target at the service people know as `service`.
    #[must_use]
    pub fn at_service(mut self, service: impl Into<String>) -> Self {
        self.service = Some(service.into());
        self
    }

    /// The target serving the model known everywhere as `model`.
    #[must_use]
    pub fn serving(mut self, model: impl Into<String>) -> Self {
        self.same_model = Some(model.into());
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
            .field("service", &self.service)
            .field("same_model", &self.same_model)
            .field("efforts", &self.efforts)
            .field("default_effort", &self.default_effort)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use glave_ports::testing::ScriptedModel;

    use super::*;

    /// A target these tests only ask what it would be sent.
    fn plain() -> Target {
        Target::new(Arc::new(ScriptedModel::new(Vec::new())), "p", "m")
    }

    fn reasons(default: Option<&str>) -> Target {
        let levels = ["high", "medium", "low"].map(str::to_owned).to_vec();
        plain().with_efforts(levels, default.map(str::to_owned))
    }

    #[test]
    fn a_target_is_sent_its_model_s_default_when_asked_for_none() {
        assert_eq!(reasons(Some("medium")).effort_for(None), Some("medium"));
    }

    #[test]
    fn the_level_asked_for_wins_over_the_default() {
        assert_eq!(
            reasons(Some("medium")).effort_for(Some("high")),
            Some("high")
        );
        assert_eq!(reasons(None).effort_for(Some("low")), Some("low"));
    }

    #[test]
    fn a_level_the_target_does_not_offer_is_never_sent() {
        let target = reasons(Some("medium"));
        assert_eq!(
            target.effort_for(Some("max")),
            Some("medium"),
            "its default"
        );
        assert_eq!(reasons(None).effort_for(Some("max")), None);
        assert_eq!(plain().effort_for(Some("high")), None, "it offers none");
        let odd = plain().with_efforts(vec!["high".into()], Some("max".into()));
        assert_eq!(odd.effort_for(None), None, "a default it does not offer");
    }

    #[test]
    fn a_target_whose_default_is_unknown_is_sent_none() {
        assert_eq!(reasons(None).effort_for(None), None);
        assert_eq!(plain().effort_for(None), None);
    }
}
