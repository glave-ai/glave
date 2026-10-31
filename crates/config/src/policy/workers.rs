//! Workers in the two files: where new threads work (`on`, a choice), the backend workers run
//! on (`[workers]` in `config.toml`), and what an organization allows them (`[workers]` in
//! `policy.toml`: limits, which only narrow).

use std::path::PathBuf;

use glave_ports::computer::{Isolation, Limits, Network};
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
    Kubernetes,
}

impl Backend {
    pub fn name(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Kubernetes => "kubernetes",
        }
    }
}

/// `[workers]` in `config.toml`: where workers run, and the glave that goes into them.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkersFile {
    pub backend: Backend,
    /// A Linux build of glave for the backend's processor.
    pub glave: PathBuf,
    /// Docker's socket, when not the usual one (Docker only).
    pub socket: Option<PathBuf>,
    /// The kubeconfig, when not `KUBECONFIG`, `~/.kube/config` or the service account glave
    /// runs as (Kubernetes only).
    pub kubeconfig: Option<PathBuf>,
    /// The namespace workers run in, when not the kubeconfig's (Kubernetes only).
    pub namespace: Option<String>,
    /// The runtime class workers run under, gVisor's or Kata's (Kubernetes only).
    pub runtime_class: Option<String>,
}

/// `[computers]` in the server's `config.toml`: each person gets a computer of their own on the
/// cluster `[workers]` reaches (`backend = "kubernetes"`), from this image, with a disk of
/// this size kept while it sleeps (`specs/computers.md`, "A person's computer in the cloud").
/// Unset, threads work on the server's own computer.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputersFile {
    /// The image every person's computer runs, pinned by digest (`name@sha256:...`).
    pub image: String,
    /// Each person's disk, in GB; [`ComputersFile::DISK_GB`] unless set.
    pub disk_gb: Option<u32>,
    /// Whether commands there run in glave's sandbox, on unless set to `false`: a cluster's
    /// runtime where it cannot start (runc) leaves the Pod as their only wall
    /// (`specs/sandbox.md`, "On a person's computer").
    pub sandbox: Option<bool>,
}

impl ComputersFile {
    /// A person's disk when `disk_gb` is not set: an organization's own cluster's default.
    pub const DISK_GB: u32 = 50;
}

impl WorkersFile {
    /// The first key set that `backend` does not use, for refusing it by name.
    pub fn misplaced(&self) -> Option<&'static str> {
        let kubernetes = [
            ("kubeconfig", self.kubeconfig.is_some()),
            ("namespace", self.namespace.is_some()),
            ("runtime_class", self.runtime_class.is_some()),
        ];
        let docker = [("socket", self.socket.is_some())];
        let others: &[(&'static str, bool)] = match self.backend {
            Backend::Docker => &kubernetes,
            Backend::Kubernetes => &docker,
        };
        others.iter().find(|(_, set)| *set).map(|(key, _)| *key)
    }
}

/// How far apart from their host workers must be, as policy writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IsolationRule {
    /// A container is enough.
    Container,
    /// A sandbox or virtual machine of its own: a Kubernetes runtime class.
    Vm,
}

impl IsolationRule {
    pub fn name(self) -> &'static str {
        match self {
            Self::Container => "container",
            Self::Vm => "vm",
        }
    }

    fn isolation(self) -> Isolation {
        match self {
            Self::Container => Isolation::Container,
            Self::Vm => Isolation::Vm,
        }
    }
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
    /// How far apart from their host workers must be.
    pub isolation: Option<IsolationRule>,
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
        if let Some(isolation) = self.isolation {
            parts.push(format!("isolation = {:?}", isolation.name()));
        }
        format!("[workers] {}", parts.join(", "))
    }
}

/// The limits of every `[workers]` rule at once: the smallest of each, the least network, and
/// the most isolation.
pub(super) fn limits<'a>(rules: impl IntoIterator<Item = &'a WorkerRules>) -> Limits {
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
        if let Some(isolation) = rule.isolation {
            limits.isolation = limits.isolation.max(isolation.isolation());
        }
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
            isolation: Some(IsolationRule::Vm),
            ..WorkerRules::default()
        };
        let c = WorkerRules {
            isolation: Some(IsolationRule::Container),
            ..WorkerRules::default()
        };
        let limits = limits([&a, &b, &c]);
        assert_eq!(limits.isolation, Isolation::Vm, "the most isolation wins");
        assert_eq!(limits.cpus, Some(2));
        assert_eq!(limits.memory_gb, Some(8));
        assert_eq!(limits.max_minutes, Some(60));
        assert_eq!(limits.network, Some(Network::Off));
        assert_eq!(
            b.describe(),
            "[workers] cpu = 2, max_minutes = 60, network = \"off\", isolation = \"vm\""
        );
    }
}
