//! The credits ledger's records (`specs/gateway.md`, "Credits"): one stream of the
//! organization's own, on a server that keeps credits. `credits.added`: money paid in, once per
//! payment, by its `reference`. `credits.limit.set`: the most the organization may spend in a
//! month, the latest replacing every one before it. `credits.charged`: what something the
//! organization used cost, by its kind, and whose work it was. The balance is everything added
//! less everything charged.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Body, Payload, parse};
use crate::id::PrincipalId;
use crate::money::Money;
use crate::payload::principal::{Principal, open_kind};

pub(crate) const ADDED: &str = "credits.added";
pub(crate) const LIMIT_SET: &str = "credits.limit.set";
pub(crate) const CHARGED: &str = "credits.charged";

open_kind! {
    /// What a charge paid for.
    pub enum CostKind {
        /// A model call, at the provider's reported cost.
        Model => "model",
    }
}

/// Money paid into the organization's credits: `amount`, from the payment `reference` names,
/// which adds it once however often it is sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreditsAdded {
    pub amount: Money,
    pub reference: String,
    pub by: Principal,
}

/// The most the organization may spend in a calendar month (UTC), in place of the server's
/// own default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreditsLimitSet {
    pub limit: Money,
    pub by: Principal,
}

/// What something the organization used cost, taken from its credits: its `kind`, and the
/// person whose work it was, when anyone's was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreditsCharged {
    pub kind: CostKind,
    pub amount: Money,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub person: Option<PrincipalId>,
}

impl Payload for CreditsAdded {
    const TYPE: &'static str = ADDED;
    const VERSION: u32 = 1;
}

impl Payload for CreditsLimitSet {
    const TYPE: &'static str = LIMIT_SET;
    const VERSION: u32 = 1;
}

impl Payload for CreditsCharged {
    const TYPE: &'static str = CHARGED;
    const VERSION: u32 = 1;
}

/// The credits ledger's events, or `None` for a type or version that is none of them.
pub(crate) fn decode(
    kind: &str,
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<Body>> {
    if version != 1 {
        return None;
    }
    Some(match kind {
        ADDED => parse(data).map(Body::CreditsAdded),
        LIMIT_SET => parse(data).map(Body::CreditsLimitSet),
        CHARGED => parse(data).map(Body::CreditsCharged),
        _ => return None,
    })
}
