//! The accent as a client draws it.

use serde::{Deserialize, Serialize};

use super::Color;

/// The accent as it is drawn, in each of light and dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Shades {
    pub light: Color,
    pub dark: Color,
}
