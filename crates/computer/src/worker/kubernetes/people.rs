//! The people of a deployment and their computers on one cluster (`specs/computers.md`, "A
//! person's computer in the cloud"): each person's computer woken when they open the app or a
//! turn needs it, once however many ask at the same time, and the workspace of their home on
//! it handed to whatever works there.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use glave_ports::computer::{
    ComputerError, Computers, Environment, Home, Image, Limits, Network, Sources, WorkspaceInfo,
};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::Kubernetes;
use super::home::PersonComputer;
use super::objects::PERSON_HOME;
use crate::worker::limits::pinned;
use crate::{WorkerComputers, WorkerCredentials};

/// How long a wake may take before whoever waits on it gives up: a Pod waits up to five
/// minutes for a node and has two more to take glave in, so minutes, never more.
const WAKE_WAIT: Duration = Duration::from_mins(10);

/// Where one person's computer is.
#[derive(Debug, Clone)]
enum State {
    Asleep,
    Waking,
    /// Awake: the workspace of its home.
    Awake(WorkspaceInfo),
    /// The last wake failed, and why; the next one tries again.
    Failed(String),
}

/// One person's computer, and the wake running for it, if one is.
struct Person {
    computers: Arc<WorkerComputers>,
    state: watch::Sender<State>,
    waking: Mutex<Option<JoinHandle<()>>>,
}

/// Every person's computer on one cluster, made from one environment within one set of
/// limits, each with a disk of the same size. Dropping it stops the wakes still running; a
/// computer already awake stays so, and sleeps on its own.
pub struct People {
    cluster: Kubernetes,
    environment: Environment,
    limits: Limits,
    disk_gb: u32,
    held: Mutex<BTreeMap<String, Arc<Person>>>,
}

impl std::fmt::Debug for People {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("People")
            .field("cluster", &self.cluster)
            .field("disk_gb", &self.disk_gb)
            .finish_non_exhaustive()
    }
}

impl People {
    /// People's computers on `cluster`, each running `image` (pinned by digest) with the
    /// network on, within `limits`, with a disk of `disk_gb`. A computer is one person's for as
    /// long as they use it, so a worker's time limit and count do not apply to it.
    pub fn new(
        cluster: Kubernetes,
        image: &str,
        limits: &Limits,
        disk_gb: u32,
    ) -> Result<Self, ComputerError> {
        if !pinned(image) {
            return Err(ComputerError::Refused(
                format!(
                    "people's computers run an image pinned by digest (name@sha256:...), not \
                     {image:?}"
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
        let environment = Environment {
            image: Image::Oci(image.to_owned()),
            network: Some(Network::On),
            ..Environment::host()
        };
        Ok(Self {
            cluster,
            environment,
            limits: Limits {
                max_minutes: None,
                per_person: None,
                ..limits.clone()
            },
            disk_gb,
            held: Mutex::new(BTreeMap::new()),
        })
    }

    /// The key a person's computer is named by: the random parts of their organization's id
    /// and of their own, never a name or an email, so two people never share one.
    pub fn key(org: uuid::Uuid, person: uuid::Uuid) -> String {
        let tail = |id: uuid::Uuid, digits: usize| {
            let hex = id.simple().to_string();
            hex.get(hex.len().saturating_sub(digits)..)
                .unwrap_or(&hex)
                .to_owned()
        };
        format!("{}-{}", tail(org, 12), tail(person, 20))
    }

    /// The computer of the person `key` names, made the first time it is asked for.
    fn person(&self, key: &str) -> Result<Arc<Person>, ComputerError> {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(person) = held.get(key) {
            return Ok(Arc::clone(person));
        }
        let computer = PersonComputer::new(self.cluster.clone(), key, self.disk_gb)?;
        let computers = WorkerComputers::on(
            Box::new(computer),
            self.limits.clone(),
            WorkerCredentials::None,
        );
        let person = Arc::new(Person {
            computers: Arc::new(computers),
            state: watch::Sender::new(State::Asleep),
            waking: Mutex::new(None),
        });
        held.insert(key.to_owned(), Arc::clone(&person));
        Ok(person)
    }

    /// Starts waking the computer of the person `key` names, unless it is awake or waking
    /// already, and returns without waiting.
    pub fn wake(&self, key: &str) -> Result<(), ComputerError> {
        let person = self.person(key)?;
        let starts = person.state.send_if_modified(|state| match state {
            State::Asleep | State::Failed(_) => {
                *state = State::Waking;
                true
            }
            State::Waking | State::Awake(_) => false,
        });
        if starts {
            let task = tokio::spawn(woken(Arc::clone(&person), self.environment.clone()));
            let mut waking = person.waking.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(before) = waking.replace(task) {
                before.abort();
            }
        }
        Ok(())
    }

    /// The computer of the person `key` names, woken if it sleeps, and the workspace of their
    /// home on it: once awake, at once.
    pub async fn home(&self, key: &str) -> Result<Home, ComputerError> {
        self.wake(key)?;
        let person = self.person(key)?;
        let mut state = person.state.subscribe();
        let woke = tokio::time::timeout(
            WAKE_WAIT,
            state.wait_for(|state| !matches!(state, State::Waking)),
        )
        .await;
        let now = match woke {
            Ok(Ok(state)) => state.clone(),
            Ok(Err(_)) => State::Failed("its wake was stopped".into()),
            Err(_) => State::Failed(format!(
                "it was still waking after {}",
                glave_ports::spoken(WAKE_WAIT)
            )),
        };
        match now {
            State::Awake(home) => Ok(Home {
                computers: Arc::clone(&person.computers) as Arc<dyn Computers>,
                home,
            }),
            State::Failed(why) => Err(ComputerError::Failed(format!(
                "your computer could not wake: {why}"
            ))),
            State::Asleep | State::Waking => Err(ComputerError::Failed(
                "your computer went to sleep as it woke".into(),
            )),
        }
    }
}

impl Drop for People {
    fn drop(&mut self) {
        let held = self.held.get_mut().unwrap_or_else(PoisonError::into_inner);
        for person in held.values() {
            let mut waking = person.waking.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(task) = waking.take() {
                task.abort();
            }
        }
    }
}

/// Wakes `person`'s computer from `environment` and opens their home on it, then says how it
/// went to whoever waits.
async fn woken(person: Arc<Person>, environment: Environment) {
    let woke = async {
        let computer = person.computers.create(&environment).await?;
        let home = Sources::Folder {
            path: PERSON_HOME.into(),
        };
        let opened = person.computers.open(computer.id, &home).await?;
        Ok::<_, ComputerError>(State::Awake(opened))
    };
    let state = woke
        .await
        .unwrap_or_else(|error| State::Failed(error.message().to_owned()));
    person.state.send_replace(state);
}

#[cfg(test)]
mod tests;
