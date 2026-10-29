//! The people of a deployment and their computers on one cluster (`specs/computers.md`, "A
//! person's computer in the cloud"): each person's computer woken when they open the app or a
//! turn needs it, once however many ask at the same time, and the workspace of their home on
//! it handed to whatever works there; kept awake while a turn or task holds it. A computer
//! sleeps on its own once nothing has asked it anything for 10 minutes and no command runs
//! there in the background (`computer serve --idle`); the next ask finds it ended and wakes it
//! again.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

use glave_ports::computer::{
    ComputerError, Computers, Environment, Home, Image, Limits, Network, Sources,
    State as ComputerState, WorkspaceInfo,
};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::Kubernetes;
use super::home::PersonComputer;
use super::objects::{IDLE_MINUTES, PERSON_HOME};
use crate::worker::limits::pinned;
use crate::{WorkerComputers, WorkerCredentials};

/// How long a wake may take before whoever waits on it gives up: a Pod waits up to five
/// minutes for a node and has two more to take glave in, so minutes, never more.
const WAKE_WAIT: Duration = Duration::from_mins(10);
/// How long after a failed wake the app opening does not start another: a broken image or a
/// full cluster is not tried again at every list the app reads. A turn that needs the
/// computer always tries again.
const RETRY_AFTER: Duration = Duration::from_mins(1);
/// The most people whose computers one server keeps track of at once; past it, those asleep
/// are let go of, and found again when they come back.
const MAX_PEOPLE: usize = 4_096;
/// The most time between asks of a computer a turn or task holds, so it never counts as idle
/// while one runs: a quarter of its idle limit, at most a minute.
const HEARTBEAT: Duration = Duration::from_mins(1);

/// Where one person's computer is.
#[derive(Debug, Clone)]
enum State {
    Asleep,
    Waking,
    /// Awake: the workspace of its home.
    Awake(WorkspaceInfo),
    /// The last wake failed, when, and why; the next one tries again.
    Failed(Instant, String),
}

impl State {
    /// Whether it is no longer waking: awake, or its wake failed.
    fn settled(&self) -> bool {
        !matches!(self, Self::Waking)
    }
}

/// One person's computer, and the wake running for it, if one is.
struct Person {
    computers: Arc<WorkerComputers>,
    state: watch::Sender<State>,
    waking: Mutex<Option<JoinHandle<()>>>,
}

/// A turn or task using a person's computer: while it is kept, the computer is asked how it is
/// once a minute, which keeps it from counting as idle. Dropping it stops the asking.
#[must_use = "the computer is kept awake only while this is kept"]
pub struct Holding(JoinHandle<()>);

impl Drop for Holding {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Every person's computer on one cluster, made from one environment within one set of
/// limits, each with a disk of the same size. Dropping it stops the wakes still running; a
/// computer already awake stays so, and sleeps on its own.
pub struct People {
    cluster: Kubernetes,
    environment: Environment,
    limits: Limits,
    disk_gb: u32,
    /// The minutes a computer goes unasked before it sleeps on its own.
    idle_minutes: u32,
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
            idle_minutes: IDLE_MINUTES,
            held: Mutex::new(BTreeMap::new()),
        })
    }

    /// Computers that sleep on their own after `minutes` unasked rather than the spec's 10, as a
    /// test's do.
    #[cfg(test)]
    pub(crate) fn idle_after(mut self, minutes: u32) -> Self {
        self.idle_minutes = minutes;
        self
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
        let computer = PersonComputer::new(self.cluster.clone(), key, self.disk_gb)?
            .idle_after(self.idle_minutes);
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
        if held.len() >= MAX_PEOPLE {
            // Kept: any awake or waking, and any something still holds, such as a turn.
            held.retain(|_, person| {
                Arc::strong_count(person) > 1
                    || !matches!(*person.state.borrow(), State::Asleep | State::Failed(..))
            });
        }
        held.insert(key.to_owned(), Arc::clone(&person));
        Ok(person)
    }

    /// Starts waking the computer of the person `key` names, unless it is awake or waking
    /// already, or its last wake failed less than a minute ago, and returns without
    /// waiting.
    pub fn wake(&self, key: &str) -> Result<(), ComputerError> {
        self.start(&self.person(key)?, Some(RETRY_AFTER));
        Ok(())
    }

    /// Starts a wake unless the computer is awake or waking, or its last wake failed less
    /// than `retry_after` ago.
    fn start(&self, person: &Arc<Person>, retry_after: Option<Duration>) {
        let starts = person.state.send_if_modified(|state| match state {
            State::Failed(at, _) if retry_after.is_some_and(|after| at.elapsed() < after) => false,
            State::Asleep | State::Failed(..) => {
                *state = State::Waking;
                true
            }
            State::Waking | State::Awake(_) => false,
        });
        if starts {
            let task = tokio::spawn(woken(Arc::clone(person), self.environment.clone()));
            let mut waking = person.waking.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(before) = waking.replace(task) {
                before.abort();
            }
        }
    }

    /// Keeps the computer of the person `key` names from sleeping while what it answers is
    /// kept: for a turn or a task. It does not wake it; while it is awake, it is asked how it is
    /// once a minute.
    pub fn hold(&self, key: &str) -> Result<Holding, ComputerError> {
        let person = self.person(key)?;
        let every = HEARTBEAT.min(Duration::from_mins(u64::from(self.idle_minutes)) / 4);
        Ok(Holding(tokio::spawn(async move {
            loop {
                tokio::time::sleep(every).await;
                let awake = match &*person.state.borrow() {
                    State::Awake(home) => Some(home.computer),
                    _ => None,
                };
                if let Some(computer) = awake {
                    let _ = person.computers.status(computer).await;
                }
            }
        })))
    }

    /// The computer of the person `key` names, woken if it sleeps, and the workspace of their
    /// home on it: once awake, at once. One that ended on its own (idle, or its node went) is
    /// let go of and woken again.
    pub async fn home(&self, key: &str) -> Result<Home, ComputerError> {
        let person = self.person(key)?;
        let was = person.state.borrow().clone();
        if let State::Awake(home) = was
            && !gone(&person, &home).await
        {
            return Ok(Home {
                computers: Arc::clone(&person.computers) as Arc<dyn Computers>,
                home,
            });
        }
        self.start(&person, None);
        let mut state = person.state.subscribe();
        let woke = tokio::time::timeout(WAKE_WAIT, state.wait_for(State::settled)).await;
        let now = match woke {
            Ok(Ok(state)) => state.clone(),
            Ok(Err(_)) => State::Failed(Instant::now(), "its wake was stopped".into()),
            Err(_) => State::Failed(
                Instant::now(),
                format!(
                    "it was still waking after {}",
                    glave_ports::spoken(WAKE_WAIT)
                ),
            ),
        };
        match now {
            State::Awake(home) => Ok(Home {
                computers: Arc::clone(&person.computers) as Arc<dyn Computers>,
                home,
            }),
            State::Failed(_, why) => Err(ComputerError::Failed(format!(
                "your computer could not wake: {why}"
            ))),
            State::Asleep | State::Waking => Err(ComputerError::Failed(
                "your computer went to sleep as it woke".into(),
            )),
        }
    }
}

/// Whether `person`'s computer, awake with `home`, has ended on its own; if so, what is left of
/// it is removed and it counts asleep, so the next ask wakes it again.
async fn gone(person: &Person, home: &WorkspaceInfo) -> bool {
    match person.computers.status(home.computer).await {
        Ok(info) if info.state == ComputerState::Running => false,
        _ => {
            let _ = person.computers.destroy(home.computer).await;
            person.state.send_if_modified(|state| match state {
                State::Awake(awake) if awake.computer == home.computer => {
                    *state = State::Asleep;
                    true
                }
                _ => false,
            });
            true
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
        .unwrap_or_else(|error| State::Failed(Instant::now(), error.message().to_owned()));
    person.state.send_replace(state);
}

#[cfg(test)]
mod tests;
