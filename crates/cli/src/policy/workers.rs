//! Workers in the two files: where new threads work (`on`, a choice), the backend workers run
//! on (`[workers]` in `config.toml`), and what an organization allows them (`[workers]` in
//! `policy.toml`: limits, which only narrow).

use std::path::PathBuf;

use glave_computer::Limits;
use glave_ports::Network;
use serde::Deserialize;

/// Where new threads work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum On {
    /// In the folder glave starts in, on this machine.
    #[default]
    Here,
    /// On a worker of their own: a disposable computer from the project's environment.
    Cloud,
}

impl On {
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "here" => Some(Self::Here),
            "cloud" => Some(Self::Cloud),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Here => "here",
            Self::Cloud => "cloud",
        }
    }
}

/// The backends workers run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Backend {
    Docker,
}

/// `[workers]` in `config.toml`: where workers run, and the glave that goes into them.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkersFile {
    pub backend: Backend,
    /// A Linux build of glave for the backend's processor.
    pub glave: PathBuf,
    /// Docker's socket, when not the usual one.
    pub socket: Option<PathBuf>,
}

/// The network a worker may have, as policy writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkRule {
    Off,
    Allowlist,
    On,
}

impl NetworkRule {
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Allowlist => "allowlist",
            Self::On => "on",
        }
    }

    fn network(self) -> Network {
        match self {
            Self::Off => Network::Off,
            Self::Allowlist => Network::Allowlist,
            Self::On => Network::On,
        }
    }
}

/// `[workers]` in `policy.toml`: what workers may be. Every key is optional.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerRules {
    /// The most CPUs a worker may have.
    pub cpu: Option<u32>,
    pub memory_gb: Option<u32>,
    /// How long a worker may take new work, in minutes.
    pub max_minutes: Option<u32>,
    /// How many workers one person may have at once.
    pub per_person: Option<u32>,
    pub network: Option<NetworkRule>,
}

impl WorkerRules {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The rule as written, for naming it.
    pub fn describe(&self) -> String {
        let numbers = [
            ("cpu", self.cpu),
            ("memory_gb", self.memory_gb),
            ("max_minutes", self.max_minutes),
            ("per_person", self.per_person),
        ];
        let mut parts: Vec<String> = numbers
            .iter()
            .filter_map(|(key, value)| value.map(|value| format!("{key} = {value}")))
            .collect();
        if let Some(network) = self.network {
            parts.push(format!("network = {:?}", network.name()));
        }
        format!("[workers] {}", parts.join(", "))
    }
}

/// The limits of every `[workers]` rule at once: the smallest of each, and the least network.
pub fn limits<'a>(rules: impl IntoIterator<Item = &'a WorkerRules>) -> Limits {
    let smallest = |a: Option<u32>, b: Option<u32>| match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    let least = |a: Option<NetworkRule>, b: Option<NetworkRule>| match (a, b) {
        (Some(NetworkRule::Off), _) | (_, Some(NetworkRule::Off)) => Some(NetworkRule::Off),
        (Some(NetworkRule::Allowlist), _) | (_, Some(NetworkRule::Allowlist)) => {
            Some(NetworkRule::Allowlist)
        }
        (a, b) => a.or(b),
    };
    let mut network = None;
    let mut limits = Limits::default();
    for rule in rules {
        limits.cpus = smallest(limits.cpus, rule.cpu);
        limits.memory_gb = smallest(limits.memory_gb, rule.memory_gb);
        limits.max_minutes = smallest(limits.max_minutes, rule.max_minutes);
        limits.per_person = smallest(limits.per_person, rule.per_person);
        network = least(network, rule.network);
    }
    limits.network = network.map(NetworkRule::network);
    limits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_take_the_smallest_and_the_least_network() {
        let a = WorkerRules {
            cpu: Some(4),
            memory_gb: Some(8),
            network: Some(NetworkRule::On),
            ..WorkerRules::default()
        };
        let b = WorkerRules {
            cpu: Some(2),
            max_minutes: Some(60),
            network: Some(NetworkRule::Off),
            ..WorkerRules::default()
        };
        let limits = limits([&a, &b]);
        assert_eq!(limits.cpus, Some(2));
        assert_eq!(limits.memory_gb, Some(8));
        assert_eq!(limits.max_minutes, Some(60));
        assert_eq!(limits.network, Some(Network::Off));
        assert_eq!(
            b.describe(),
            "[workers] cpu = 2, max_minutes = 60, network = \"off\""
        );
    }
}
