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
    /// The providers a router (OpenRouter) passes calls on to, as it lists them: where each is
    /// based. Empty for any other service.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<CatalogProvider>,
}

/// A provider a router passes calls on to, as the router lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogProvider {
    /// What the router's routing preferences call it, such as `deepinfra`.
    pub slug: String,
    /// Its name, such as `DeepInfra`.
    pub name: String,
    /// The country it is based in, as an ISO 3166-1 alpha-2 code such as `US`, when the
    /// router says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headquarters: Option<String>,
    /// The countries its datacenters are in, when the router says; absent when it does not,
    /// which says nothing of where they are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub datacenters: Option<Vec<String>>,
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
    /// At a router (OpenRouter), each provider that serves it, as the router lists them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub endpoints: Vec<CatalogEndpoint>,
}

/// One provider serving a model at a router, as the router's list of the model's endpoints
/// describes it. What the router does not say is absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogEndpoint {
    /// The provider's slug, as its [`CatalogProvider`] has it.
    pub provider: String,
    /// How the weights it serves are stored, as the router says: `fp8`, `bf16`, `int4`;
    /// absent when it says `unknown` or nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantization: Option<String>,
    /// Its context window, in tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<u64>,
    /// The longest answer it gives, in tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<TokenPrice>,
    /// Whether the router lists it as keeping nothing of what it is sent: zero data
    /// retention.
    #[serde(default, skip_serializing_if = "is_false")]
    pub zdr: bool,
    /// How many tokens a second it answered with, the median of the router's last half hour
    /// when it was listed, when the router says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<u64>,
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
            endpoints: Vec::new(),
        }
    }
}

/// How precisely a provider serves a model's weights, from how it says they are stored.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Precision {
    /// It does not say.
    Unstated,
    /// Four bits a weight: `int4`, `fp4`, `mxfp4`, `nvfp4`.
    FourBit,
    /// Six bits a weight: `fp6`.
    SixBit,
    /// Eight bits a weight: `int8`, `fp8`, `mxfp8`.
    EightBit,
    /// The weights as they were trained: `fp16`, `bf16`, `fp32`.
    Full,
}

impl Precision {
    /// The ways of storing weights a router names, by precision.
    const NAMED: [(&'static str, Self); 11] = [
        ("int4", Self::FourBit),
        ("fp4", Self::FourBit),
        ("mxfp4", Self::FourBit),
        ("nvfp4", Self::FourBit),
        ("fp6", Self::SixBit),
        ("int8", Self::EightBit),
        ("fp8", Self::EightBit),
        ("mxfp8", Self::EightBit),
        ("fp16", Self::Full),
        ("bf16", Self::Full),
        ("fp32", Self::Full),
    ];

    /// How a router names the ways of storing weights at this precision or better: none for
    /// [`Precision::Unstated`], which asks for nothing.
    pub fn at_least(self) -> Vec<&'static str> {
        if self == Self::Unstated {
            return Vec::new();
        }
        let enough = Self::NAMED
            .iter()
            .filter(|(_, precision)| *precision >= self);
        enough.map(|(name, _)| *name).collect()
    }

    /// The precision of weights stored as `quantization` says, as a router names it.
    pub fn of(quantization: Option<&str>) -> Self {
        let named = quantization.map(str::to_ascii_lowercase);
        Self::NAMED
            .iter()
            .find(|(name, _)| named.as_deref() == Some(*name))
            .map_or(Self::Unstated, |(_, precision)| *precision)
    }
}

/// What a model's tokens cost, in millionths of a US dollar per million tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TokenPrice {
    pub input: u64,
    pub output: u64,
    /// Input read from the service's cache, when it prices that apart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<u64>,
}

/// How a model reasons: the levels of effort it offers, in its service's own names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Reasoning {
    /// The levels it offers, as its service lists them, such as `high`, `medium`, `low`, and
    /// `none` where reasoning can be turned off. Empty when it offers no choice.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub efforts: Vec<String>,
    /// The level it uses when none is asked for, one of `efforts`, when its service says.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_precision_is_read_from_how_weights_are_stored_and_ranked_by_it() {
        assert_eq!(Precision::of(Some("fp8")), Precision::EightBit);
        assert_eq!(Precision::of(Some("BF16")), Precision::Full);
        assert_eq!(Precision::of(Some("nvfp4")), Precision::FourBit);
        assert_eq!(Precision::of(Some("fp6")), Precision::SixBit);
        assert_eq!(Precision::of(Some("unknown")), Precision::Unstated);
        assert_eq!(Precision::of(None), Precision::Unstated);
        assert!(Precision::Full > Precision::EightBit && Precision::EightBit > Precision::FourBit);
        assert_eq!(Precision::Full.at_least(), ["fp16", "bf16", "fp32"]);
        assert_eq!(
            Precision::EightBit.at_least(),
            ["int8", "fp8", "mxfp8", "fp16", "bf16", "fp32"]
        );
        assert!(
            Precision::Unstated.at_least().is_empty(),
            "asks for nothing"
        );
    }
}
