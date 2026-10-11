//! Memory's settings (`specs/memory.md`, "Defaults and policy"): the person's choice
//! (`memory = "on"` or `"off"` in their own `config.toml`), the space's setting
//! (`space_memory`, which a project's file may choose), and an organization's `[memory]`
//! constraints, which only narrow.

use crate::error::refuse;
use glave_memory::SpaceMemory;
use serde::Deserialize;

use crate::policy::choices::{Choices, Source};
use crate::policy::rules::{Constraints, Kind};

/// `memory` in `config.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryChoice {
    On,
    Off,
}

/// A space's memory setting as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpaceChoice {
    Shared,
    Isolated,
    Off,
}

impl SpaceChoice {
    pub fn setting(self) -> SpaceMemory {
        match self {
            Self::Shared => SpaceMemory::Shared,
            Self::Isolated => SpaceMemory::Isolated,
            Self::Off => SpaceMemory::Off,
        }
    }
}

/// `[memory]` in `policy.toml`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryRules {
    /// `false`: no memory anywhere in the organization.
    pub enabled: Option<bool>,
    /// `false`: no personal memory; spaces may only be `isolated` or `off`.
    pub personal: Option<bool>,
    /// The space settings allowed.
    pub spaces: Option<Vec<SpaceChoice>>,
    /// Memories not saved or edited for this many days are forgotten by glave.
    pub retention_days: Option<u32>,
    /// `glave`, or the name of an MCP server that serves memory instead.
    pub server: Option<String>,
}

impl MemoryRules {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The rule as written: `[memory] enabled = false, personal = false`.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(enabled) = self.enabled {
            parts.push(format!("enabled = {enabled}"));
        }
        if let Some(personal) = self.personal {
            parts.push(format!("personal = {personal}"));
        }
        if let Some(spaces) = &self.spaces {
            let names: Vec<String> = spaces
                .iter()
                .map(|s| format!("{:?}", s.setting().as_str()))
                .collect();
            parts.push(format!("spaces = [{}]", names.join(", ")));
        }
        if let Some(days) = self.retention_days {
            parts.push(format!("retention_days = {days}"));
        }
        if let Some(server) = &self.server {
            parts.push(format!("server = {server:?}"));
        }
        format!("[memory] {}", parts.join(", "))
    }

    /// Whether memory is served by another server than glave's own, which is not built yet.
    pub fn other_server(&self) -> bool {
        self.server
            .as_deref()
            .is_some_and(|server| server != "glave")
    }
}

/// What memory runs with, within the constraints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemorySettings {
    /// Whether glave reads and writes memory for this person at all.
    pub on: bool,
    /// Whether personal memory is allowed.
    pub personal: bool,
    /// This space's setting.
    pub space: SpaceMemory,
    pub retention_days: Option<u32>,
}

/// The memory settings of `choices` under `constraints`, or why they are not allowed.
pub fn settle(choices: &Choices, constraints: &Constraints) -> crate::Result<MemorySettings> {
    let rules: Vec<_> = constraints
        .rules()
        .iter()
        .filter_map(|rule| match &rule.kind {
            Kind::Memory(memory) => Some((rule, memory)),
            _ => None,
        })
        .collect();
    let mut on = choices.memory.value;
    if let Some((rule, _)) = rules.iter().find(|(_, m)| m.enabled == Some(false)) {
        if on && choices.memory.from != Source::Default {
            refuse!(
                "memory = \"on\" (from {}) is not allowed by {rule}",
                choices.memory.from
            );
        }
        on = false;
    }
    let personal_rule = rules.iter().find(|(_, m)| m.personal == Some(false));
    let allowed: Vec<SpaceMemory> = [SpaceMemory::Shared, SpaceMemory::Isolated, SpaceMemory::Off]
        .into_iter()
        .filter(|space| *space != SpaceMemory::Shared || personal_rule.is_none())
        .filter(|space| {
            rules.iter().all(|(_, m)| {
                m.spaces
                    .as_ref()
                    .is_none_or(|spaces| spaces.iter().any(|s| s.setting() == *space))
            })
        })
        .collect();
    let chosen = &choices.space_memory;
    let space = if allowed.contains(&chosen.value) {
        chosen.value
    } else if chosen.from == Source::Default {
        allowed.first().copied().unwrap_or(SpaceMemory::Off)
    } else {
        let rules: Vec<String> = rules.iter().map(|(rule, _)| rule.to_string()).collect();
        refuse!(
            "space_memory = {:?} (from {}) is not allowed by {}",
            chosen.value.as_str(),
            chosen.from,
            rules.join(", and ")
        );
    };
    Ok(MemorySettings {
        on,
        personal: personal_rule.is_none(),
        space,
        retention_days: rules.iter().filter_map(|(_, m)| m.retention_days).min(),
    })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::policy::choices::{Layer, Level, resolve};
    use crate::policy::files;
    use crate::policy::tests::constraints;

    fn chosen(config: &str) -> Choices {
        let path = PathBuf::from("/home/ada/.glave/config.toml");
        resolve(&[Layer {
            values: files::config(config, Path::new(&path)).unwrap(),
            level: Level::File(path),
        }])
    }

    #[test]
    fn memory_is_on_by_default_and_off_by_the_persons_choice() {
        let none = constraints("version = 1");
        let on = settle(&chosen(""), &none).unwrap();
        assert!(on.on);
        assert_eq!(on.space, SpaceMemory::Shared);
        assert!(!settle(&chosen("memory = \"off\""), &none).unwrap().on);
    }

    #[test]
    fn a_policy_only_narrows_memory() {
        let no_personal =
            constraints("version = 1\n[memory]\npersonal = false\nretention_days = 90");
        let settled = settle(&chosen("memory = \"on\""), &no_personal).unwrap();
        assert!(!settled.personal);
        assert_eq!(
            settled.space,
            SpaceMemory::Isolated,
            "the default gives way"
        );
        assert_eq!(settled.retention_days, Some(90));
        let shared = settle(&chosen("space_memory = \"shared\""), &no_personal);
        assert!(
            shared.is_err(),
            "a choice outside the policy is an error naming it"
        );
        let disabled = constraints("version = 1\n[memory]\nenabled = false");
        assert!(settle(&chosen("memory = \"on\""), &disabled).is_err());
        assert!(!settle(&chosen(""), &disabled).unwrap().on);
        let only_off = constraints("version = 1\n[memory]\nspaces = [\"off\"]");
        assert_eq!(
            settle(&chosen(""), &only_off).unwrap().space,
            SpaceMemory::Off
        );
    }

    #[test]
    fn another_memory_server_is_refused_until_it_is_built() {
        let other = constraints("version = 1\n[memory]\nserver = \"corp-memory\"");
        assert_ne!(other.not_enforced().len(), 0);
        let glave = constraints("version = 1\n[memory]\nserver = \"glave\"");
        assert_eq!(glave.not_enforced().len(), 0);
    }
}
