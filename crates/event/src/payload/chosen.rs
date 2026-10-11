//! `model.chosen`: someone chose the model a thread's turns are sent to. The latest one is the
//! thread's choice; a thread with none uses the deployment's default model.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};
use crate::payload::principal::Principal;

pub(crate) const TYPE: &str = "model.chosen";

/// The thread's turns go to `model` from now on, until another is chosen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelChosen {
    /// The model's id, as the session API's `model/list` names it.
    pub model: String,
    /// Who chose it. Absent: the thread's `for`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

impl Payload for ModelChosen {
    const TYPE: &'static str = TYPE;
    const VERSION: u32 = 1;
}

pub(crate) fn decode(version: u32, data: &RawValue) -> Option<serde_json::Result<ModelChosen>> {
    (version == 1).then(|| parse(data))
}
