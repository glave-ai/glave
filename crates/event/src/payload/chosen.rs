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

/// What the agent may do without asking the person (`specs/approvals.md`, "Modes"): the one
/// choice a person makes about it, its mode. Ordered by caution, the most cautious least, so a
/// policy's cap is the highest mode it allows. Each comes to a sandbox and how much asks
/// wherever it runs (`glave-harness`).
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
    /// Ask first: commands run in a sandbox that writes nothing of the project, and every
    /// change asks first.
    Ask,
    /// The agent works on its own in the project, or the chat's folder: commands run in the
    /// sandbox and nothing inside it asks; risky actions ask or go to the judge. Where no
    /// sandbox can run, every change and command asks instead.
    #[default]
    Auto,
    /// No sandbox, and nothing ever asks, risky actions included; what would show the model a
    /// secret is declined instead. For work the person fully trusts.
    FullAccess,
}

impl Permissions {
    /// Every mode, in the order a person is offered them: the recommended one first.
    pub const ALL: [Self; 3] = [Self::Auto, Self::Ask, Self::FullAccess];

    /// The mode `name` names, as config and the API write it.
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.name() == name)
    }

    /// The mode's name, as config and the API write it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Auto => "auto",
            Self::FullAccess => "full-access",
        }
    }
}

/// The thread's turns run with `permissions` from now on, until another mode is chosen.
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
    fn modes_are_named_as_config_and_the_api_write_them_and_ordered_by_caution() {
        assert_eq!(Permissions::default(), Permissions::Auto);
        assert_eq!(
            Permissions::ALL[0],
            Permissions::Auto,
            "the default is offered first"
        );
        for retired in ["workspace", "read-only"] {
            assert_eq!(Permissions::named(retired), None);
        }
        for mode in Permissions::ALL {
            assert_eq!(Permissions::named(mode.name()), Some(mode));
            assert_eq!(serde_json::to_value(mode).unwrap(), mode.name());
        }
        assert!(Permissions::Ask < Permissions::Auto);
        assert!(Permissions::Auto < Permissions::FullAccess);
    }
}
