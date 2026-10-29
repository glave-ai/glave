//! The people of a deployment and their computers on one cluster (`specs/computers.md`, "A
//! person's computer in the cloud"): each person's computer woken when they open the app or a
//! turn needs it, once however many ask at the same time, and the workspace of their home on
//! it handed to whatever works there; and put to sleep once it has been idle for 10 minutes:
//! no turn or task holding it, and no command running there in the background.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

use glave_ports::computer::{
    ComputerError, ComputerId, Computers, Environment, Home, Image, Limits, Network, Sources,
    State as ComputerState, WorkspaceInfo,
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
/// How long after a failed wake the app opening does not start another: a broken image or a
/// full cluster is not tried again at every list the app reads. A turn that needs the
/// computer always tries again.
const RETRY_AFTER: Duration = Duration::from_mins(1);
/// The most people whose computers one server keeps track of at once; past it, those asleep
/// are let go of, and found again when they come back.
const MAX_PEOPLE: usize = 4_096;
/// How long a computer is idle before it sleeps (`specs/computers.md`, decision 4).
const IDLE: Duration = Duration::from_mins(10);
/// The most time between looks at which computers are idle.
const IDLE_CHECK: Duration = Duration::from_mins(1);
/// How many times a turn's ask goes round a computer falling asleep as it asked.
const ASKS: usize = 3;

/// Where one person's computer is.
#[derive(Debug, Clone)]
enum State {
    Asleep,
    Waking,
    /// Awake: the workspace of its home.
    Awake(WorkspaceInfo),
    /// The last wake failed, when, and why; the next one tries again.
    Failed(Instant, String),
    /// Going to sleep: its Pod being removed. A wake waits until it is asleep.
    Sleeping,
}

impl State {
    /// Whether it is neither waking nor going to sleep.
    fn settled(&self) -> bool {
        !matches!(self, Self::Waking | Self::Sleeping)
    }
}

/// One person's computer, the wake running for it, if one is, and what keeps it awake.
struct Person {
    computers: Arc<WorkerComputers>,
    state: watch::Sender<State>,
    waking: Mutex<Option<JoinHandle<()>>>,
    using: Mutex<Using>,
}

/// What uses a person's computer: how many turns and tasks hold it, and since when none has.
struct Using {
    holds: usize,
    quiet_since: Instant,
}

impl Person {
    /// Notes that something used the computer just now.
    fn touch(&self) {
        let mut using = self.using.lock().unwrap_or_else(PoisonError::into_inner);
        using.quiet_since = Instant::now();
    }

    /// Whether nothing has held the computer for `idle`.
    fn quiet_for(&self, idle: Duration) -> bool {
        let using = self.using.lock().unwrap_or_else(PoisonError::into_inner);
        using.holds == 0 && using.quiet_since.elapsed() >= idle
    }
}

/// A turn or task using a person's computer: it does not sleep while one is kept.
#[must_use = "the computer is kept awake only while this is kept"]
pub struct Holding(Arc<Person>);

impl Drop for Holding {
    fn drop(&mut self) {
        let mut using = self.0.using.lock().unwrap_or_else(PoisonError::into_inner);
        using.holds = using.holds.saturating_sub(1);
        using.quiet_since = Instant::now();
    }
}

/// The people's computers one server tracks, by key.
type Tracked = Arc<Mutex<BTreeMap<String, Arc<Person>>>>;

/// Every person's computer on one cluster, made from one environment within one set of
/// limits, each with a disk of the same size. Dropping it stops the wakes still running; a
/// computer already awake stays so, and sleeps on its own.
pub struct People {
    cluster: Kubernetes,
    environment: Environment,
    limits: Limits,
    disk_gb: u32,
    held: Tracked,
    /// Puts computers idle long enough to sleep, stopped with this.
    sleeper: JoinHandle<()>,
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
        Self::idle_after(cluster, image, limits, disk_gb, IDLE)
    }

    /// People's computers as [`People::new`] makes them, each put to sleep once idle for
    /// `idle`.
    pub(crate) fn idle_after(
        cluster: Kubernetes,
        image: &str,
        limits: &Limits,
        disk_gb: u32,
        idle: Duration,
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
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            ComputerError::Failed("people's computers are kept from within a runtime".into())
        })?;
        let held = Tracked::default();
        let sleeper = runtime.spawn(sleeping(Arc::clone(&held), idle));
        Ok(Self {
            cluster,
            environment,
            limits: Limits {
                max_minutes: None,
                per_person: None,
                ..limits.clone()
            },
            disk_gb,
            held,
            sleeper,
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
            using: Mutex::new(Using {
                holds: 0,
                quiet_since: Instant::now(),
            }),
        });
        if held.len() >= MAX_PEOPLE {
            held.retain(|_, person| {
                !matches!(*person.state.borrow(), State::Asleep | State::Failed(..))
            });
        }
        held.insert(key.to_owned(), Arc::clone(&person));
        Ok(person)
    }

    /// Starts waking the computer of the person `key` names, unless it is awake or waking
    /// already, or its last wake failed less than a minute ago, and returns without
    /// waiting.
    pub fn wake(&self, key: &str) -> Result<(), ComputerError> {
        self.start(key, Some(RETRY_AFTER))
    }

    /// Starts a wake unless the computer is awake or waking, or its last wake failed less
    /// than `retry_after` ago.
    fn start(&self, key: &str, retry_after: Option<Duration>) -> Result<(), ComputerError> {
        let person = self.person(key)?;
        let starts = person.state.send_if_modified(|state| match state {
            State::Failed(at, _) if retry_after.is_some_and(|after| at.elapsed() < after) => false,
            State::Asleep | State::Failed(..) => {
                *state = State::Waking;
                true
            }
            State::Waking | State::Awake(_) | State::Sleeping => false,
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

    /// Keeps the computer of the person `key` names from sleeping while what it answers is
    /// kept: for a turn or a task. It does not wake it.
    pub fn hold(&self, key: &str) -> Result<Holding, ComputerError> {
        let person = self.person(key)?;
        person
            .using
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .holds += 1;
        Ok(Holding(person))
    }

    /// The computer of the person `key` names, woken if it sleeps, and the workspace of their
    /// home on it: once awake, at once. One that went away on its own (its glave ended idle,
    /// or its node did) is woken again, and one going to sleep as it is asked for, once asleep.
    pub async fn home(&self, key: &str) -> Result<Home, ComputerError> {
        let person = self.person(key)?;
        person.touch();
        for _ in 0..ASKS {
            let was = person.state.borrow().clone();
            if let State::Awake(home) = was
                && !gone(&person, &home).await
            {
                return Ok(Home {
                    computers: Arc::clone(&person.computers) as Arc<dyn Computers>,
                    home,
                });
            }
            self.start(key, None)?;
            match settled(&person).await {
                State::Awake(home) => {
                    return Ok(Home {
                        computers: Arc::clone(&person.computers) as Arc<dyn Computers>,
                        home,
                    });
                }
                State::Failed(_, why) => {
                    return Err(ComputerError::Failed(format!(
                        "your computer could not wake: {why}"
                    )));
                }
                State::Asleep | State::Waking | State::Sleeping => {}
            }
        }
        Err(ComputerError::Failed(
            "your computer kept going to sleep as it woke".into(),
        ))
    }
}

/// `person`'s computer once it is neither waking nor going to sleep, within [`WAKE_WAIT`].
async fn settled(person: &Person) -> State {
    let mut state = person.state.subscribe();
    let woke = tokio::time::timeout(WAKE_WAIT, state.wait_for(State::settled)).await;
    match woke {
        Ok(Ok(state)) => state.clone(),
        Ok(Err(_)) => State::Failed(Instant::now(), "its wake was stopped".into()),
        Err(_) => State::Failed(
            Instant::now(),
            format!(
                "it was still waking after {}",
                glave_ports::spoken(WAKE_WAIT)
            ),
        ),
    }
}

/// Puts each computer of `held` to sleep once it has been idle for `idle`: nothing has held
/// it, or asked for it, and no command runs there in the background. Looks at least every
/// minute, until it is stopped.
#[expect(
    clippy::infinite_loop,
    reason = "runs until People is dropped, which aborts it"
)]
async fn sleeping(held: Tracked, idle: Duration) {
    let every = IDLE_CHECK.min(idle / 4).max(Duration::from_millis(10));
    loop {
        tokio::time::sleep(every).await;
        let people: Vec<Arc<Person>> = held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect();
        for person in people {
            let awake = match &*person.state.borrow() {
                State::Awake(home) => Some(home.computer),
                _ => None,
            };
            if let Some(computer) = awake
                && person.quiet_for(idle)
                && !person.computers.running(computer).await
            {
                asleep(&person, computer).await;
            }
        }
    }
}

/// Puts `person`'s computer to sleep: its Pod removed, its disk kept. Nothing happens if it
/// was asked for meanwhile, or already woke again on another computer.
async fn asleep(person: &Person, computer: ComputerId) {
    let sleeps = person.state.send_if_modified(|state| match state {
        State::Awake(home) if home.computer == computer && person.quiet_for(Duration::ZERO) => {
            *state = State::Sleeping;
            true
        }
        _ => false,
    });
    if !sleeps {
        return;
    }
    // One that could not be removed now is removed by the next wake, which removes any Pod
    // left from before, or ends on its own once idle.
    let _ = person.computers.destroy(computer).await;
    person.state.send_replace(State::Asleep);
}

/// Whether `person`'s computer, awake with `home`, has gone away on its own; if so, it is let
/// go of and counted asleep, so the next ask wakes it again.
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
        self.sleeper.abort();
        let held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
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
    person.touch();
    person.state.send_replace(state);
}

#[cfg(test)]
mod tests;
