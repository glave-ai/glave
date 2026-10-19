//! What a worker may be, as an organization's policy limits it (`[workers]` in `policy.toml`):
//! policy sets the limits, and the workers check each environment against them.

use super::environment::Network;

/// Policy's limits on workers (`[workers]` in `policy.toml`). Every one is optional.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Limits {
    /// The most CPUs a worker may have; also its size when its environment names none.
    pub cpus: Option<u32>,
    /// The most memory, in GB; also its size when its environment names none.
    pub memory_gb: Option<u32>,
    /// How long a worker may run, in minutes, before it refuses new work.
    pub max_minutes: Option<u32>,
    /// How many workers one person may have at once.
    pub per_person: Option<u32>,
    /// The most network a worker may have; `None` allows it all. An allow-list is not
    /// enforced yet, so a policy asking for one is refused before workers start.
    pub network: Option<Network>,
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
