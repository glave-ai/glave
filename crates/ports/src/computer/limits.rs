//! What a worker may be, as an organization's policy limits it (`[workers]` in `policy.toml`):
//! policy sets the limits, and the workers check each environment against them.

use super::sandbox::NetworkMode;

/// Policy's limits on workers (`[workers]` in `policy.toml`). Every one is optional.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Limits {
    /// The most CPUs a worker may have; also its size when its environment names none.
    pub cpus: Option<u32>,
    /// The most memory, in GB; also its size when its environment names none.
    pub memory_gb: Option<u32>,
    /// The most network a worker may have; `None` allows it all. A worker can't be held to
    /// `guarded` yet: policy refuses it when read, and a worker given it gets no network.
    pub network: Option<NetworkMode>,
    /// How far apart from its host a worker must be kept.
    pub isolation: Isolation,
}

/// How a worker is kept apart from the machine it runs on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Isolation {
    /// A container: its own namespaces and limits on a shared kernel.
    #[default]
    Container,
    /// A sandbox with a kernel of its own (gVisor) or a virtual machine (Kata), from a
    /// Kubernetes runtime class; untrusted code needs it.
    Vm,
}
