//! The settings ledger's record of the language the person reads glave's words in
//! (`specs/session.md`, "Languages"). `language.set`: the person's choice; the latest replaces
//! every one before it, and with none, or none, their client's language counts.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};

pub(crate) const SET: &str = "language.set";

/// The language the person chose, as a BCP 47 tag (`en`, `ar-XB`); none to follow their
/// client's again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanguageSet {
    pub language: Option<String>,
}

impl Payload for LanguageSet {
    const TYPE: &'static str = SET;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_set(version: u32, data: &RawValue) -> Option<serde_json::Result<LanguageSet>> {
    (version == 1).then(|| parse(data))
}
