//! The models a server can run, as `model/list` shows them, so a client can offer a choice:
//! each model once, whichever services can run it, with what its provider says it can do and
//! costs.

mod names;

use glave_event::{Reasoning, TokenPrice};
use serde::{Deserialize, Serialize};

/// One model a thread may be sent to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ModelInfo {
    /// What `thread/start` and `turn/start` take as `model`: the name the gateway knows it by.
    pub id: String,
    /// Its own name, without its maker's, such as `Claude Haiku 5.5`: the settings', else its
    /// provider's, else written from its id as its maker writes it.
    pub name: String,
    /// Who made it, such as `Anthropic`: as its provider says, else from its id's prefix
    /// (`anthropic/…`); absent when neither says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maker: Option<String>,
    /// Whether threads that choose no model use this one. Exactly one model is the default.
    pub default: bool,
    /// The services that run it, in a person's words (`OpenRouter`, `Anthropic`, a server's
    /// address), in the order glave tries them: the first, then each after it only when the
    /// one before fails.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<String>,
    /// Its context window, in tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<u64>,
    /// The longest answer it gives, in tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output: Option<u64>,
    /// Whether it reads images.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<bool>,
    /// What its tokens cost at the first of its providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<TokenPrice>,
    /// How it reasons, when it can: the levels of effort it offers, in its provider's names,
    /// and as `default` the level its first provider is sent when a thread chooses none, the
    /// one that provider's catalog says the model uses unasked. No `default` when glave sends
    /// none, and the model reasons as its maker set it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<Reasoning>,
}

impl ModelInfo {
    /// `id`, named from it, with nothing else known, not the default.
    pub fn new(id: impl Into<String>) -> Self {
        let id = id.into();
        Self {
            name: names::own_name(&id),
            maker: names::maker_of(&id),
            id,
            default: false,
            providers: Vec::new(),
            context: None,
            max_output: None,
            images: None,
            price: None,
            reasoning: None,
        }
    }

    /// Named `name`, as a provider's listing or the settings give it: one that says its maker
    /// first (`Anthropic: Claude Haiku 5.5`) names both; a blank one changes nothing.
    #[must_use]
    pub fn named(mut self, name: &str) -> Self {
        let name = name.trim();
        match name.split_once(": ") {
            Some((maker, own)) if !maker.trim().is_empty() && !own.trim().is_empty() => {
                self.maker = Some(maker.trim().to_owned());
                own.trim().clone_into(&mut self.name);
            }
            _ if !name.is_empty() => name.clone_into(&mut self.name),
            _ => {}
        }
        self
    }
}
