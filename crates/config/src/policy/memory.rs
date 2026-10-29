//! Memory's settings (`specs/memory.md`, "Defaults and policy"): the project's setting
//! (`project_memory`, which a project's file may choose) and an organization's `[memory]`
//! constraints, which only narrow. Whether the person wants memory is their own choice, kept
//! in their settings ledger, not a file.

use crate::error::refuse;
use glave_ports::memories::ProjectMemory;
use serde::Deserialize;

use crate::policy::choices::{Choices, Source};
use crate::policy::rules::{Constraints, Kind};

/// A project's memory setting as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectChoice {
    Shared,
    Isolated,
    Off,
}

impl ProjectChoice {
    pub fn setting(self) -> ProjectMemory {
        match self {
            Self::Shared => ProjectMemory::Shared,
            Self::Isolated => ProjectMemory::Isolated,
            Self::Off => ProjectMemory::Off,
        }
    }
}

/// `[memory]` in `policy.toml`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryRules {
    /// `false`: no memory anywhere in the organization.
    pub enabled: Option<bool>,
    /// `false`: no personal memory; projects may only be `isolated` or `off`.
    pub personal: Option<bool>,
    /// The project settings allowed.
    pub projects: Option<Vec<ProjectChoice>>,
    /// Memories not saved or edited for this many days are forgotten by glave.
    pub retention_days: Option<u32>,
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
        if let Some(projects) = &self.projects {
            let names: Vec<String> = projects
                .iter()
                .map(|s| format!("{:?}", s.setting().as_str()))
                .collect();
            parts.push(format!("projects = [{}]", names.join(", ")));
        }
        if let Some(days) = self.retention_days {
            parts.push(format!("retention_days = {days}"));
        }
        format!("[memory] {}", parts.join(", "))
    }
}

/// What memory runs with, within the constraints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemorySettings {
    /// Whether the policy lets glave read and write memory at all.
    pub on: bool,
    /// Whether personal memory is allowed.
    pub personal: bool,
    /// This project's setting.
    pub project: ProjectMemory,
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
    let on = rules.iter().all(|(_, m)| m.enabled != Some(false));
    let personal_rule = rules.iter().find(|(_, m)| m.personal == Some(false));
    let allowed: Vec<ProjectMemory> = [
        ProjectMemory::Shared,
        ProjectMemory::Isolated,
        ProjectMemory::Off,
    ]
    .into_iter()
    .filter(|project| *project != ProjectMemory::Shared || personal_rule.is_none())
    .filter(|project| {
        rules.iter().all(|(_, m)| {
            m.projects
                .as_ref()
                .is_none_or(|projects| projects.iter().any(|s| s.setting() == *project))
        })
    })
    .collect();
    let chosen = &choices.project_memory;
    let project = if allowed.contains(&chosen.value) {
        chosen.value
    } else if chosen.from == Source::Default {
        allowed.first().copied().unwrap_or(ProjectMemory::Off)
    } else {
        let rules: Vec<String> = rules.iter().map(|(rule, _)| rule.to_string()).collect();
        refuse!(
            "project_memory = {:?} (from {}) is not allowed by {}",
            chosen.value.as_str(),
            chosen.from,
            rules.join(", and ")
        );
    };
    Ok(MemorySettings {
        on,
        personal: personal_rule.is_none(),
        project,
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
    fn memory_is_on_and_shared_by_default() {
        let none = constraints("version = 1");
        let on = settle(&chosen(""), &none).unwrap();
        assert!(on.on);
        assert_eq!(on.project, ProjectMemory::Shared);
    }

    #[test]
    fn a_policy_only_narrows_memory() {
        let no_personal =
            constraints("version = 1\n[memory]\npersonal = false\nretention_days = 90");
        let settled = settle(&chosen(""), &no_personal).unwrap();
        assert!(!settled.personal);
        assert_eq!(
            settled.project,
            ProjectMemory::Isolated,
            "the default gives way"
        );
        assert_eq!(settled.retention_days, Some(90));
        let shared = settle(&chosen("project_memory = \"shared\""), &no_personal);
        assert!(
            shared.is_err(),
            "a choice outside the policy is an error naming it"
        );
        let disabled = constraints("version = 1\n[memory]\nenabled = false");
        assert!(!settle(&chosen(""), &disabled).unwrap().on);
        let only_off = constraints("version = 1\n[memory]\nprojects = [\"off\"]");
        assert_eq!(
            settle(&chosen(""), &only_off).unwrap().project,
            ProjectMemory::Off
        );
    }
}
