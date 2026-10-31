//! The settings ledger's record of how a person asks routers (OpenRouter) to choose who serves
//! their models (`specs/gateway.md`, "A person's filters"). `filters.set`: the person's
//! filters; the latest replaces every one before it, and with none a router chooses as it
//! does unasked, among the providers that never train on what they are sent.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};
use crate::payload::catalog::Precision;

pub(crate) const SET: &str = "filters.set";

/// Which of a model's providers a router prefers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Sort {
    /// The lowest price.
    Price,
    /// The most tokens a second.
    Speed,
}

/// Who serves one model, as a person chose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Serve {
    /// Whichever provider within the filters is cheapest, or fastest, when it is called.
    Sort(Sort),
    /// This provider alone, by slug.
    Provider(String),
}

/// The most a provider's tokens may cost, in millionths of a US dollar per million tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaxPrice {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<u64>,
}

/// A person's filters among the providers a router passes their calls on to. What is not
/// chosen does not filter.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Filters {
    /// Which provider to prefer when several may serve: the router's own balance unless
    /// chosen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<Sort>,
    /// The least precision a provider may serve a model's weights at; a provider that does
    /// not say its precision serves no filter but none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precision: Option<Precision>,
    /// Only providers based in these countries (ISO 3166-1 alpha-2 codes), each of their
    /// datacenters too where the router names them; anywhere when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub based_in: Vec<String>,
    /// Only providers that keep nothing of what they are sent (zero data retention).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keeps_nothing: bool,
    /// The most a provider's tokens may cost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_price: Option<MaxPrice>,
    /// Providers never to use, by slug.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub avoid: Vec<String>,
    /// Who serves a model, by the model's id, where the person chose for it apart from
    /// `sort`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub serves: BTreeMap<String, Serve>,
}

/// A filter of a person's that hides a provider of a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Hides {
    /// It is based, or runs, outside the countries chosen, or the router does not say where
    /// it is based.
    BasedIn,
    /// It serves the model's weights at less than the precision chosen, or does not say.
    Precision,
    /// It may keep what it is sent, where only providers that keep nothing are chosen.
    KeepsSome,
    /// Its tokens cost more than the most chosen, or the router does not say what they cost.
    Price,
    /// The person chose to avoid it.
    Avoided,
}

/// The person's filters, as they set them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FiltersSet {
    pub filters: Filters,
}

impl Payload for FiltersSet {
    const TYPE: &'static str = SET;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_set(version: u32, data: &RawValue) -> Option<serde_json::Result<FiltersSet>> {
    (version == 1).then(|| parse(data))
}
