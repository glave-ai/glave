//! What someone chose for a thread's turns: `model.chosen`, the model they are sent to, and
//! `approval.mode.chosen`, how much glave asks before acting in them. The latest of each type
//! is the thread's choice; a thread with none uses the deployment's own.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};
use crate::payload::principal::Principal;

pub(crate) const TYPE: &str = "model.chosen";
pub(crate) const APPROVAL: &str = "approval.mode.chosen";

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

/// How much glave asks the person before acting (`specs/approvals.md`, "Modes"). Secret files
/// ask in every mode.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    /// The cautious mode: ask before every change and every command.
    Ask,
    /// Changes and commands go through: in the sandbox when it is on, as the person when not;
    /// risky ones still ask. glave chooses it by default only where commands run in the
    /// sandbox.
    #[default]
    Auto,
}

impl ApprovalMode {
    /// The mode `name` names, as config and the API write it.
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "ask" => Some(Self::Ask),
            "auto" => Some(Self::Auto),
            _ => None,
        }
    }

    /// The mode's name, as config and the API write it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Auto => "auto",
        }
    }
}

/// The thread's turns run in approval mode `mode` from now on, until another is chosen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalModeChosen {
    pub mode: ApprovalMode,
    /// Who chose it. Absent: the thread's `for`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

impl Payload for ApprovalModeChosen {
    const TYPE: &'static str = APPROVAL;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_approval(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ApprovalModeChosen>> {
    (version == 1).then(|| parse(data))
}

#[cfg(test)]
mod tests {
    use super::ApprovalMode::{self, Ask, Auto};

    #[test]
    fn modes_are_named_as_config_and_the_api_write_them() {
        assert_eq!(ApprovalMode::default(), Auto);
        assert_eq!(ApprovalMode::named("edits"), None, "retired");
        for mode in [Ask, Auto] {
            assert_eq!(ApprovalMode::named(mode.name()), Some(mode));
            assert_eq!(serde_json::to_value(mode).unwrap(), mode.name());
        }
    }
}
