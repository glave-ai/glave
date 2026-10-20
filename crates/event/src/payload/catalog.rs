//! The catalog ledger's record (`specs/gateway.md`, "Catalogs"): what each service a person
//! connected says about its models, as its own model list gives it, so glave never hardcodes
//! a model's context, output limit, price or reasoning levels. `catalog.refreshed` holds every
//! service's catalog; the latest replaces the ones before it.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};
use crate::envelope::At;
use crate::payload::provider::ProviderKind;

pub(crate) const REFRESHED: &str = "catalog.refreshed";

/// The catalogs of the services connected, each as it was last listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogRefreshed {
    pub catalogs: Vec<Catalog>,
}

/// One service's models, as its model list described them when it was last read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub kind: ProviderKind,
    /// The server's API, for a `compatible` service.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// When the list was read.
    pub listed: At,
    /// The models glave can talk with there, newest first.
    pub models: Vec<CatalogModel>,
}

/// One model, as far as its service describes it. What the service does not say is absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogModel {
    /// The id the service takes as `model`.
    pub id: String,
    /// The service's name for it, such as `Anthropic: Claude Haiku 5.5`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Its context window, in tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<u64>,
    /// The longest answer it gives, in tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output: Option<u64>,
    /// Whether it reads images.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<TokenPrice>,
    /// How it reasons, when it can.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<Reasoning>,
}

impl CatalogModel {
    /// `id`, described no further.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: None,
            context: None,
            max_output: None,
            images: None,
            price: None,
            reasoning: None,
        }
    }
}

/// What a model's tokens cost, in millionths of a US dollar per million tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenPrice {
    pub input: u64,
    pub output: u64,
    /// Input read from the service's cache, when it prices that apart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<u64>,
}

/// How a model reasons: the levels of effort it offers, in its service's own names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reasoning {
    /// The levels it offers, as its service lists them, such as `high`, `medium`, `low`, and
    /// `none` where reasoning can be turned off. Empty when it offers no choice.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub efforts: Vec<String>,
    /// The level it uses when none is asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// Whether it always reasons.
    #[serde(default, skip_serializing_if = "is_false")]
    pub mandatory: bool,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if"
)]
fn is_false(value: &bool) -> bool {
    !*value
}

impl Payload for CatalogRefreshed {
    const TYPE: &'static str = REFRESHED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_refreshed(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<CatalogRefreshed>> {
    (version == 1).then(|| parse(data))
}
