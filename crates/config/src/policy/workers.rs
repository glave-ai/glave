//! The cluster in the two files: where people's computers run (`[workers]` in `config.toml`,
//! with `[computers]`), and what an organization allows them (`[workers]` in `policy.toml`:
//! limits, which only narrow).

use std::path::PathBuf;

use glave_ports::computer::{Isolation, Limits, NetworkMode};
use serde::Deserialize;

/// `[workers]` in `config.toml`: the Kubernetes cluster people's computers run on, and the
/// glave that goes into them.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkersFile {
    /// A Linux build of glave for the cluster's processor.
    pub glave: PathBuf,
    /// The kubeconfig, when not `KUBECONFIG`, `~/.kube/config` or the service account glave
    /// runs as.
    pub kubeconfig: Option<PathBuf>,
    /// The namespace they run in, when not the kubeconfig's.
    pub namespace: Option<String>,
    /// The runtime class they run under, gVisor's or Kata's.
    pub runtime_class: Option<String>,
}

/// `[computers]` in the server's `config.toml`: each person gets a computer of their own on the
/// cluster `[workers]` reaches, from this image, with a disk of this size kept while it sleeps
/// (`specs/computers.md`, "A person's computer in the cloud"). Unset, threads work on the
/// server's own computer.
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

/// How far apart from their host people's computers must be, as policy writes it.
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

/// `[workers]` in `policy.toml`: what people's computers may be. Every key is optional.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerRules {
    /// The most CPUs a computer may have.
    pub cpu: Option<u32>,
    pub memory_gb: Option<u32>,
    /// The most network a computer may have: `open` or `offline`.
    #[serde(default, deserialize_with = "held_to")]
    pub network: Option<NetworkMode>,
    /// How far apart from their host they must be.
    pub isolation: Option<IsolationRule>,
}

impl WorkerRules {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The rule as written, for naming it.
    pub fn describe(&self) -> String {
        let numbers = [("cpu", self.cpu), ("memory_gb", self.memory_gb)];
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
    let mut limits = Limits::default();
    for rule in rules {
        limits.cpus = smallest(limits.cpus, rule.cpu);
        limits.memory_gb = smallest(limits.memory_gb, rule.memory_gb);
        limits.network = smallest(limits.network, rule.network);
        if let Some(isolation) = rule.isolation {
            limits.isolation = limits.isolation.max(isolation.isolation());
        }
    }
    limits
}

/// A network people's computers can be held to, refused when read otherwise: no computer can
/// be kept to the hosts allowed yet, and a rule every wake refused would leave people none.
fn held_to<'de, D: serde::Deserializer<'de>>(reader: D) -> Result<Option<NetworkMode>, D::Error> {
    match NetworkMode::deserialize(reader)? {
        NetworkMode::Guarded => Err(serde::de::Error::custom(
            "network = \"guarded\" is not enforced on people's computers yet: use \"open\" or \
             \"offline\"",
        )),
        mode => Ok(Some(mode)),
    }
}

/// The smaller of two limits, either of which may be unset.
fn smallest<T: Ord>(a: Option<T>, b: Option<T>) -> Option<T> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_take_the_smallest_and_the_least_network() {
        let a = WorkerRules {
            cpu: Some(4),
            memory_gb: Some(8),
            network: Some(NetworkMode::Open),
            ..WorkerRules::default()
        };
        let b = WorkerRules {
            cpu: Some(2),
            network: Some(NetworkMode::Offline),
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
        assert_eq!(limits.network, Some(NetworkMode::Offline));
        assert_eq!(
            b.describe(),
            "[workers] cpu = 2, network = \"offline\", isolation = \"vm\""
        );
    }

    #[test]
    fn the_least_network_is_the_smallest_mode_and_guarded_is_refused_when_read() {
        use NetworkMode::{Guarded, Offline, Open};
        assert!(Offline < Guarded && Guarded < Open);
        assert_eq!(smallest(Some(Guarded), Some(Open)), Some(Guarded));
        assert_eq!(smallest(None, Some(Open)), Some(Open));
        let read = |text: &str| toml::from_str::<WorkerRules>(text);
        assert_eq!(read("network = \"open\"").unwrap().network, Some(Open));
        let refused = read("network = \"guarded\"").unwrap_err().to_string();
        assert!(refused.contains("not enforced"), "{refused}");
        assert!(read("network = \"on\"").is_err(), "the old words are gone");
    }
}
