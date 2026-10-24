//! What someone chose for a thread's turns: `model.chosen`, the model they are sent to,
//! `effort.chosen`, how much that model reasons, and `permissions.chosen`, what the agent may
//! do in them without asking. The latest of each type is the thread's choice; a thread with
//! none uses the deployment's own.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};
use crate::payload::principal::Principal;

pub(crate) const TYPE: &str = "model.chosen";
pub(crate) const PERMISSIONS: &str = "permissions.chosen";
pub(crate) const EFFORT: &str = "effort.chosen";

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

/// The thread's turns ask their model to reason at `effort` from now on, until another is
/// chosen: a level in its provider's own names, as `model/list` gives them, such as `high` or
/// `none`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffortChosen {
    pub effort: String,
    /// Who chose it. Absent: the thread's `for`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

impl Payload for EffortChosen {
    const TYPE: &'static str = EFFORT;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_effort(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<EffortChosen>> {
    (version == 1).then(|| parse(data))
}

/// What the agent may do without asking the person (`specs/approvals.md`, "Permissions"): the
/// one choice a person makes about it, of three presets, the most cautious first. Each comes to
/// a sandbox and how much asks wherever it runs (`glave-harness`); risky actions ask in every
/// preset.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Permissions {
    /// The agent looks: commands run in a sandbox that writes nothing of the project, and
    /// every change and command asks first.
    ReadOnly,
    /// The agent changes the project, or the chat's folder: commands run in the sandbox and
    /// nothing inside it asks; risky actions ask or go to the judge. Where no sandbox can run,
    /// every change and command asks instead.
    #[default]
    Workspace,
    /// No sandbox, and nothing asks but the risky actions: for work the person fully trusts.
    FullAccess,
}

impl Permissions {
    /// Every preset, the most cautious first.
    pub const ALL: [Self; 3] = [Self::ReadOnly, Self::Workspace, Self::FullAccess];

    /// The preset `name` names, as config and the API write it.
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|preset| preset.name() == name)
    }

    /// The preset's name, as config and the API write it.
    pub fn name(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::Workspace => "workspace",
            Self::FullAccess => "full-access",
        }
    }
}

/// The thread's turns run with `permissions` from now on, until another preset is chosen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionsChosen {
    pub permissions: Permissions,
    /// Who chose it. Absent: the thread's `for`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

impl Payload for PermissionsChosen {
    const TYPE: &'static str = PERMISSIONS;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_permissions(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<PermissionsChosen>> {
    (version == 1).then(|| parse(data))
}

#[cfg(test)]
mod tests {
    use super::Permissions;

    #[test]
    fn presets_are_named_as_config_and_the_api_write_them_most_cautious_first() {
        assert_eq!(Permissions::default(), Permissions::Workspace);
        assert_eq!(Permissions::named("auto"), None);
        for preset in Permissions::ALL {
            assert_eq!(Permissions::named(preset.name()), Some(preset));
            assert_eq!(serde_json::to_value(preset).unwrap(), preset.name());
        }
        assert!(Permissions::ReadOnly < Permissions::Workspace);
        assert!(Permissions::Workspace < Permissions::FullAccess);
    }
}
