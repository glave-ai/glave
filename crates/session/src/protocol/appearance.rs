//! How glave looks, as a client draws it.

use serde::{Deserialize, Serialize};

use glave_event::Sha256;

use super::{Accent, Color};

/// The accent as it is drawn, in each of light and dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Shades {
    pub light: Color,
    pub dark: Color,
}

/// What the caller's organization says of how glave looks to them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OrganizationLook {
    /// What it is called; none until someone names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Which logo it has (`brand/logo/get`), by its hash, so a client reads it once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<String>")]
    pub logo: Option<Sha256>,
    /// Its accent.
    pub accent: Accent,
    /// Whether everyone uses that accent, so the caller chooses none.
    pub locked: bool,
}

/// A logo's content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Logo {
    #[schemars(with = "String")]
    pub sha256: Sha256,
    pub media_type: String,
    /// In base64.
    pub data: String,
}
