//! A person's computer on Kubernetes (`specs/computers.md`, "A person's computer in the
//! cloud"): a PersistentVolumeClaim that is their disk and, while awake, one Pod mounting it as
//! their home, with a NetworkPolicy beside it. Sleeping removes the Pod and its policy and
//! keeps the claim; waking makes them again on the same claim, with glave and a fresh token
//! copied in. Nothing here is owned by the Pod glave runs in: a person's computer outlives any
//! one glave, and is labelled `glave.computer`, never as a worker, so no glave reaps it.

use std::sync::Arc;
use std::time::Duration;

use glave_ports::BoxFuture;
use glave_ports::computer::{ComputerError, Isolation};
use k8s_openapi::api::core::v1::PersistentVolumeClaim;
use kube::Api;
use kube::api::{DeleteParams, PostParams};
use serde_json::json;

use super::objects::{COMPUTER_LABEL, Home, IDLE_MINUTES};
use super::sizing::{Size, Sizing};
use super::{Kubernetes, POLL, failed, not_found};
use crate::token::Token;
use crate::worker::backend::{Backend, Link, Listed, Spec, Started, Stream, saying};

/// How long a removed Pod or claim has to go: the Pod's grace period (5 seconds) and its
/// volumes let go of, so seconds.
const GONE_WAIT: Duration = Duration::from_mins(1);
/// The longest key a person's computer is named by, so its objects' names stay labels.
const KEY_LENGTH: usize = 40;

/// One person's computer on a cluster: its key, the size of its disk, and how big it is.
#[derive(Debug, Clone)]
pub(crate) struct PersonComputer {
    cluster: Kubernetes,
    key: String,
    disk_gb: u32,
    /// The minutes it goes unasked before it sleeps on its own.
    idle_minutes: u32,
    /// Its CPU and memory now, and at its next wake, shared by every copy of it.
    pub(crate) sizing: Arc<Sizing>,
}

impl PersonComputer {
    /// The computer of the person `key` names on `cluster`, with a disk of `disk_gb`. The key
    /// is lowercase letters, digits and `-` (at most 40), as derived from the organization
    /// and the person's id, never from a name or an email.
    pub(crate) fn new(cluster: Kubernetes, key: &str, disk_gb: u32) -> Result<Self, ComputerError> {
        if !plain(key) {
            return Err(ComputerError::Refused(
                format!(
                    "{key:?} cannot name a person's computer: it takes lowercase letters, \
                     digits and -, at most {KEY_LENGTH}"
                )
                .into(),
            ));
        }
        if disk_gb == 0 {
            return Err(ComputerError::Refused(
                "a person's computer needs a disk of at least 1 GB"
                    .to_owned()
                    .into(),
            ));
        }
        Ok(Self {
            cluster,
            key: key.to_owned(),
            disk_gb,
            idle_minutes: IDLE_MINUTES,
            sizing: Arc::default(),
        })
    }

    /// Grows the computer awake in place to `size`, within a minute, or says why it could not.
    pub(crate) async fn resize(&self, size: Size) -> Result<(), ComputerError> {
        self.cluster.resize(&self.name(), size).await?;
        self.sizing.set(size);
        Ok(())
    }

    /// Sleeps on its own after `minutes` unasked rather than the spec's 10, as a test's does.
    #[must_use]
    pub(crate) fn idle_after(self, minutes: u32) -> Self {
        Self {
            idle_minutes: minutes,
            ..self
        }
    }

    /// The Pod's name, and its policy's.
    fn name(&self) -> String {
        format!("glave-computer-{}", self.key)
    }

    /// The claim's name: the person's disk.
    fn claim_name(&self) -> String {
        format!("glave-home-{}", self.key)
    }

    fn claims(&self) -> Api<PersistentVolumeClaim> {
        Api::namespaced(self.cluster.client.clone(), &self.cluster.namespace)
    }

    /// Makes the person's disk, unless it is there already (made by this glave before, or by
    /// another of the organization's).
    async fn claim(&self) -> Result<(), ComputerError> {
        let mut labels = self.cluster.labels.clone();
        labels.insert(COMPUTER_LABEL.to_owned(), self.key.clone());
        let claim: PersistentVolumeClaim = serde_json::from_value(json!({
            "apiVersion": "v1",
            "kind": "PersistentVolumeClaim",
            "metadata": {"name": self.claim_name(), "labels": labels},
            "spec": {
                "accessModes": ["ReadWriteOnce"],
                "resources": {"requests": {"storage": format!("{}Gi", self.disk_gb)}}
            }
        }))
        .map_err(|error| failed("disk claim", error))?;
        match self.claims().create(&PostParams::default(), &claim).await {
            Ok(_) => Ok(()),
            Err(kube::Error::Api(status)) if status.is_already_exists() => self.kept().await,
            Err(error) => Err(failed("refused the person's disk", error)),
        }
    }

    /// Checks that the disk already there is not being removed, which no Pod could mount.
    async fn kept(&self) -> Result<(), ComputerError> {
        let name = self.claim_name();
        let claim = self
            .claims()
            .get(&name)
            .await
            .map_err(|error| failed(&format!("could not read the disk {name}"), error))?;
        if claim.metadata.deletion_timestamp.is_some() {
            return Err(ComputerError::Failed(format!(
                "the disk {name} is being removed, so the computer cannot wake on it"
            )));
        }
        Ok(())
    }

    /// Wakes the computer from `spec`: its disk made if it is new, a Pod left from before
    /// removed (its glave takes a token no longer held), and a new one started on the disk.
    async fn wake(&self, spec: &Spec) -> Result<Link, ComputerError> {
        let size = self.sizing.waking();
        let spec = &Spec {
            cpu_millis: Some(size.cpu_millis),
            memory_mib: Some(size.memory_mib),
            ..spec.clone()
        };
        self.claim().await?;
        self.sleep().await?;
        let glave = self.cluster.glave().await?;
        let name = self.name();
        let claim = self.claim_name();
        let home = Home {
            key: &self.key,
            claim: &claim,
            idle_minutes: self.idle_minutes,
        };
        match self.cluster.start_on(&name, spec, &glave, Some(home)).await {
            Ok(link) => Ok(link),
            Err(error) => {
                let said = self.cluster.logs(&name).await;
                let _ = self.sleep().await;
                Err(saying(error, &said))
            }
        }
    }

    /// Puts the computer to sleep: its Pod and policy removed, and gone, so the disk is free to
    /// mount again; the disk kept. Asleep already is not an error.
    async fn sleep(&self) -> Result<(), ComputerError> {
        let name = self.name();
        // Within its grace period, not at once: the Pod's object goes only once its containers
        // have stopped and its volumes are let go of, so nothing still writes to the disk when
        // it mounts again. Its policy goes after it, so it never runs unpoliced.
        removed(&self.cluster.pods(), &name, "computer").await?;
        removed(&self.cluster.policies(), &name, "computer's network policy").await
    }

    /// Whether the computer is awake as the API server says: its Pod there, not being removed,
    /// and its container running.
    pub(crate) async fn running(&self) -> Result<bool, ComputerError> {
        super::running(&self.cluster.pods(), &self.name()).await
    }

    /// Whether its Pod ended killed for memory, as the API server says.
    pub(crate) async fn ran_out_of_memory(&self) -> bool {
        let pod = self
            .cluster
            .pods()
            .get_opt(&self.name())
            .await
            .ok()
            .flatten();
        pod.and_then(|pod| serde_json::to_value(pod).ok())
            .and_then(|pod| {
                pod.pointer("/status/containerStatuses/0/state/terminated/reason")
                    .cloned()
            })
            .is_some_and(|reason| reason == "OOMKilled")
    }

    /// Removes the computer and its disk, with everything on it.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "a person's computer is removed with the person, which comes with Remove (#221)"
        )
    )]
    pub(crate) async fn destroy(&self) -> Result<(), ComputerError> {
        self.sleep().await?;
        removed(&self.claims(), &self.claim_name(), "disk").await
    }
}

/// Whether `key` can name a person's computer: lowercase letters, digits and `-`, neither
/// first nor last, at most [`KEY_LENGTH`], so every object's name made from it is a label.
pub(super) fn plain(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= KEY_LENGTH
        && !key.starts_with('-')
        && !key.ends_with('-')
        && key
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Removes the object `name`, gracefully, and waits until it is gone. One already gone is not
/// an error.
async fn removed<K>(api: &Api<K>, name: &str, what: &str) -> Result<(), ComputerError>
where
    K: kube::Resource + Clone + serde::de::DeserializeOwned + std::fmt::Debug,
{
    match api.delete(name, &DeleteParams::default()).await {
        Ok(_) => {}
        Err(error) if not_found(&error) => {}
        Err(error) => {
            return Err(failed(
                &format!("could not remove the {what} {name}"),
                error,
            ));
        }
    }
    gone(api, name, what).await
}

/// Waits until the object `name` is gone, within [`GONE_WAIT`].
async fn gone<K>(api: &Api<K>, name: &str, what: &str) -> Result<(), ComputerError>
where
    K: kube::Resource + Clone + serde::de::DeserializeOwned + std::fmt::Debug,
{
    let mut waited = Duration::ZERO;
    loop {
        match api.get_opt(name).await {
            Ok(None) => return Ok(()),
            Ok(Some(_)) if waited >= GONE_WAIT => {
                return Err(ComputerError::Failed(format!(
                    "the {what} {name} was still there {} after it was removed",
                    glave_ports::spoken(GONE_WAIT)
                )));
            }
            Ok(Some(_)) => {
                tokio::time::sleep(POLL).await;
                waited = waited.saturating_add(POLL);
            }
            Err(error) => return Err(failed(&format!("could not read the {what} {name}"), error)),
        }
    }
}

/// A person's computer as a place a computer runs: starting it wakes it, removing it puts it
/// to sleep, and it lists no workers, since it is none.
impl Backend for PersonComputer {
    fn isolation(&self) -> Isolation {
        self.cluster.isolation()
    }

    fn start<'a>(&'a self, spec: &'a Spec) -> BoxFuture<'a, Result<Started, ComputerError>> {
        Box::pin(async move {
            let link = self.wake(spec).await?;
            Ok(Started {
                name: self.name(),
                link,
            })
        })
    }

    fn renew<'a>(
        &'a self,
        name: &'a str,
        token: &'a Token,
    ) -> BoxFuture<'a, Result<(), ComputerError>> {
        self.cluster.renew(name, token)
    }

    fn bridge<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Stream, ComputerError>> {
        self.cluster.bridge(name)
    }

    fn list(&self) -> BoxFuture<'_, Result<Vec<Listed>, ComputerError>> {
        Box::pin(std::future::ready(Ok(Vec::new())))
    }

    fn remove<'a>(&'a self, _name: &'a str) -> BoxFuture<'a, Result<(), ComputerError>> {
        Box::pin(self.sleep())
    }
}

#[cfg(test)]
pub(super) mod tests;
