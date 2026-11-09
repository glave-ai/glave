//! People's computers in the cloud (`specs/computers.md`, "A person's computer in the cloud"):
//! where the server's settings give each person a computer of their own on a Kubernetes
//! cluster (`[computers]`, on the cluster `[workers]` reaches), every thread of theirs works
//! there; without them, threads work on the server's own computer, as on a laptop.

use std::sync::Arc;

use anyhow::bail;
use glave_computer::{Kubernetes, KubernetesSettings, Leasing, People, person_sandbox};
use glave_event::PrincipalId;
use glave_harness::Homes;
use glave_ports::computer::{ComputerError, Home, Sandbox};
use glave_ports::{Holder, Leases, OrgId, OsEntropy, SystemClock};

use glave_config::policy::{Choices, ComputersFile, Constraints};

/// Each person's own computer, where the server's settings give people one (`[computers]`,
/// on the Kubernetes cluster `[workers]` reaches), within the organization's `[workers]`
/// limits, each driven only by the server holding its lease in `leases`; none otherwise, and
/// threads work on the server's own computer, as on a laptop.
pub(crate) async fn homes(
    choices: &Choices,
    constraints: &Constraints,
    leases: Arc<dyn Leases>,
) -> anyhow::Result<Option<Arc<dyn Homes>>> {
    let Some(computers) = &choices.computers else {
        return Ok(None);
    };
    let Some(cluster) = &choices.workers else {
        bail!(
            "[computers] (from {}) puts people's computers on the Kubernetes cluster [workers] \
             reaches: set [workers] glave = \"<a Linux build of glave>\" in config.toml",
            computers.from
        );
    };
    let cluster = &cluster.value;
    let kubernetes = Kubernetes::connect(KubernetesSettings {
        kubeconfig: cluster.kubeconfig.clone(),
        namespace: cluster.namespace.clone(),
        runtime_class: cluster.runtime_class.clone(),
        glave: cluster.glave.clone(),
    })
    .await?;
    let disk_gb = computers.value.disk_gb.unwrap_or(ComputersFile::DISK_GB);
    let limits = constraints.worker_limits();
    let leasing = Leasing {
        leases,
        holder: Holder::new(&SystemClock, &OsEntropy)?,
    };
    let people = People::new(
        kubernetes,
        &computers.value.image,
        &limits,
        disk_gb,
        leasing,
    )?;
    let sandbox = computers
        .value
        .sandbox
        .unwrap_or(true)
        .then(|| person_sandbox(choices.network.value));
    if sandbox.is_none()
        && let Some(rule) = constraints.sandbox_required()
    {
        bail!(
            "[computers] sandbox = false (from {}) leaves commands without the sandbox {rule} \
             requires: run people's computers where it can start, under gVisor",
            computers.from
        );
    }
    Ok(Some(Arc::new(OnCluster { people, sandbox })))
}

/// People's computers on a cluster, as the harness asks for them: each named by the person's
/// organization and id.
struct OnCluster {
    people: People,
    sandbox: Option<Sandbox>,
}

impl Homes for OnCluster {
    fn home(&self, org: OrgId, person: PrincipalId) -> Result<Home, ComputerError> {
        self.people.home(org, person)
    }

    fn hold(&self, org: OrgId, person: PrincipalId) -> Result<Box<dyn Send>, ComputerError> {
        let held = self.people.hold(org, person)?;
        Ok(Box::new(held))
    }

    fn sandbox(&self) -> Option<Sandbox> {
        self.sandbox.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Leases kept in memory, as a server alone on its store keeps them.
    fn leases() -> Arc<dyn Leases> {
        Arc::new(glave_ports::InMemoryLeases::new(Arc::new(SystemClock)))
    }

    #[test]
    fn people_s_computers_without_a_cluster_say_what_to_set() {
        use glave_config::policy::{Layer, Level, assemble};
        let dir = glave_ports::testing::TempDir::new("computers").unwrap();
        let text = format!("[computers]\nimage = \"x@sha256:{}\"\n", "0".repeat(64));
        let layer = Layer {
            level: Level::File(dir.join("config.toml")),
            values: glave_config::policy::config(&text, &dir).unwrap(),
        };
        let choices = assemble(&[layer], &[], None).choices;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let constraints = Constraints::default();
        let here = runtime
            .block_on(homes(
                &assemble(&[], &[], None).choices,
                &constraints,
                leases(),
            ))
            .unwrap();
        assert!(here.is_none(), "here by default");
        let refused = runtime
            .block_on(homes(&choices, &constraints, leases()))
            .err()
            .unwrap();
        assert!(refused.to_string().contains("[workers] glave"), "{refused}");
    }
}
