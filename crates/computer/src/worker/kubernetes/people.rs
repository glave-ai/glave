//! The people of a deployment and their computers on one cluster (`specs/computers.md`, "A
//! person's computer in the cloud"): each person's computer handed to whatever works there as
//! planned, asleep or awake, and woken only by the first call that needs it (decision 6), once
//! however many ask at the same time; kept awake while a turn or task holds it. A computer
//! sleeps on its own once nothing has asked it anything for 10 minutes and no command runs
//! there in the background (`computer serve --idle`); the next ask finds it ended and wakes it
//! again.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use glave_event::PrincipalId;
use glave_ports::OrgId;
use glave_ports::computer::{
    ComputerError, ComputerId, Computers, Environment, Home, Image, Limits, Network, Sources,
    State as ComputerState, WorkspaceId, WorkspaceInfo,
};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::Kubernetes;
use super::home::PersonComputer;
use super::objects::{IDLE_MINUTES, PERSON_HOME, PERSON_SCRATCH};
use super::sizing::{CEILING, Size};
use crate::worker::limits::pinned;
use crate::{Grow, Short, Wake, Waking, WorkerComputers, WorkerCredentials};

mod lease;

pub use lease::Leasing;

/// How long a wake may take before whoever waits on it gives up: a Pod waits up to five
/// minutes for a node and has two more to take glave in, so minutes, never more.
const WAKE_WAIT: Duration = Duration::from_mins(10);
/// How long after a failed wake a call is told it failed rather than starting another: a broken
/// image or a full cluster is not tried again at every tool call of a turn.
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
    /// The last wake failed, when, and why; a call a minute later or more tries again.
    Failed(Instant, String),
}

impl State {
    /// Whether it is no longer waking: awake, or its wake failed.
    fn settled(&self) -> bool {
        !matches!(self, Self::Waking)
    }
}

/// One person's computer, as planned and as it is, and the wake running for it, if one is.
struct Person {
    /// Whose it is, as its lease names it.
    org: OrgId,
    id: PrincipalId,
    leasing: Leasing,
    /// Its lease while this server holds it, and what renews it.
    held: Mutex<Option<lease::Held>>,
    renewing: Mutex<Option<JoinHandle<()>>>,
    /// The computer as whatever works there holds it, waking at its first call that needs it.
    planned: Home,
    /// Its Pod and disk, asked directly whether it runs when its connection fails.
    pod: PersonComputer,
    /// The largest it grows.
    ceiling: Size,
    /// Held by the one growth at a time, so two never undo each other.
    growing: tokio::sync::Mutex<()>,
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
    /// The largest a computer grows: the organization's limits, else Glave Cloud's personal
    /// plan's.
    ceiling: Size,
    /// Where each computer's lease is kept, so one server at a time drives it.
    leasing: Leasing,
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
    /// network on, within `limits`, with a disk of `disk_gb`, each driven by this server only
    /// while it holds the computer's lease (`leasing`). A computer is one person's for as long as
    /// they use it, so a worker's time limit and count do not apply to it.
    pub fn new(
        cluster: Kubernetes,
        image: &str,
        limits: &Limits,
        disk_gb: u32,
        leasing: Leasing,
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
        let ceiling = Size {
            cpu_millis: limits
                .cpus
                .map_or(CEILING.cpu_millis, |cpus| cpus.saturating_mul(1_000)),
            memory_mib: limits
                .memory_gb
                .map_or(CEILING.memory_mib, |gb| gb.saturating_mul(1_024)),
        };
        Ok(Self {
            cluster,
            environment,
            ceiling,
            limits: Limits {
                max_minutes: None,
                per_person: None,
                ..limits.clone()
            },
            disk_gb,
            idle_minutes: IDLE_MINUTES,
            leasing,
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
    pub(crate) fn key(org: uuid::Uuid, person: uuid::Uuid) -> String {
        let tail = |id: uuid::Uuid, digits: usize| {
            let hex = id.simple().to_string();
            hex.get(hex.len().saturating_sub(digits)..)
                .unwrap_or(&hex)
                .to_owned()
        };
        format!("{}-{}", tail(org, 12), tail(person, 20))
    }

    /// The computer of `person` in `org`, made the first time it is asked for.
    fn person(&self, org: OrgId, person: PrincipalId) -> Result<Arc<Person>, ComputerError> {
        let key = &Self::key(org.as_uuid(), person.as_uuid());
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(person) = held.get(key) {
            return Ok(Arc::clone(person));
        }
        let id = person;
        let computer = PersonComputer::new(self.cluster.clone(), key, self.disk_gb)?
            .idle_after(self.idle_minutes);
        let computers = WorkerComputers::on(
            Box::new(computer.clone()),
            self.limits.clone(),
            WorkerCredentials::None,
        );
        let home = planned_home(key)?;
        let ceiling = self.ceiling;
        let person = Arc::new_cyclic(|me: &std::sync::Weak<Person>| {
            let grows = std::sync::Weak::clone(me);
            let grow: Grow = Arc::new(move |short| {
                let me = std::sync::Weak::clone(&grows);
                Box::pin(async move { grow(&me.upgrade()?, short, ceiling).await })
            });
            let (me, environment) = (std::sync::Weak::clone(me), self.environment.clone());
            let wake: Wake = Arc::new(move || {
                let (me, environment) = (std::sync::Weak::clone(&me), environment.clone());
                Box::pin(async move {
                    let person = me.upgrade().ok_or_else(|| {
                        ComputerError::Failed(
                            "your computer is no longer kept by this server".into(),
                        )
                    })?;
                    awake(&person, &environment).await
                })
            });
            Person {
                org,
                id,
                leasing: self.leasing.clone(),
                held: Mutex::new(None),
                renewing: Mutex::new(None),
                planned: Waking::home(wake, Some(grow), home, PERSON_SCRATCH),
                pod: computer,
                ceiling,
                growing: tokio::sync::Mutex::new(()),
                computers: Arc::new(computers),
                state: watch::Sender::new(State::Asleep),
                waking: Mutex::new(None),
            }
        });
        if held.len() >= MAX_PEOPLE {
            // Kept: any awake or waking, and any something still holds, such as a turn or a
            // thread's workspace on it, which names it by its id, the same for a new one.
            held.retain(|_, person| {
                Arc::strong_count(person) > 1
                    || Arc::strong_count(&person.planned.computers) > 1
                    || !matches!(*person.state.borrow(), State::Asleep | State::Failed(..))
            });
        }
        held.insert(key.to_owned(), Arc::clone(&person));
        Ok(person)
    }

    /// Keeps the computer of `person` in `org` from sleeping while what it answers is kept: for
    /// a turn or a task. It does not wake it; while it is awake, it is asked how it is once a
    /// minute.
    pub fn hold(&self, org: OrgId, person: PrincipalId) -> Result<Holding, ComputerError> {
        let person = self.person(org, person)?;
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

    /// The computer of `person` in `org` and the workspace of their home on it, as planned: at
    /// once, asleep or awake. The first call on it that needs the computer wakes it, if it
    /// sleeps, and waits for that wake, which every other call asking meanwhile shares; one that
    /// ended on its own (idle, or its node went) is woken again. Another server driving it, by
    /// its lease, refuses the call in plain words.
    pub fn home(&self, org: OrgId, person: PrincipalId) -> Result<Home, ComputerError> {
        Ok(self.person(org, person)?.planned.clone())
    }
}

/// The home of a person's computer as planned before it wakes, on the computer `key` names:
/// the computer's id made from the key's digits (derived, not time-ordered as other ids are),
/// the same on every server and after every restart, so
/// whatever works there, or a project's folder made there, finds it the same computer however
/// often it slept and whichever server asks.
fn planned_home(key: &str) -> Result<WorkspaceInfo, ComputerError> {
    let digits: String = key.chars().filter(char::is_ascii_hexdigit).collect();
    let id = u128::from_str_radix(&digits, 16)
        .map_err(|_| ComputerError::Failed(format!("{key} names no person's computer")))?;
    Ok(WorkspaceInfo {
        id: WorkspaceId::from_uuid(crate::local::new_id()?),
        computer: ComputerId::from_uuid(uuid::Uuid::from_u128(id)),
        root: PERSON_HOME.into(),
        sources: Sources::Folder {
            path: PERSON_HOME.into(),
        },
        home: Some(PERSON_HOME.into()),
        temp: None,
    })
}

/// Grows `person`'s computer for what a command ran `short` of, within `ceiling`, one growth
/// at a time: a computer held at its CPU limit in place where the cluster can, else at its next
/// wake; one killed for memory, its glave with it, at its next wake, which the next call makes.
/// What to tell whoever ran the command; nothing when it did not run short.
async fn grow(person: &Arc<Person>, short: Short, ceiling: Size) -> Option<String> {
    let _growing = person.growing.lock().await;
    let now = person.pod.sizing.now();
    match short {
        Short::Cpu => {
            let bigger = now.grown(short, ceiling)?;
            if holds(person) && person.pod.resize(bigger).await.is_ok() {
                return Some(format!(
                    "glave: this computer now has {}.",
                    bigger.said(short)
                ));
            }
            person.pod.sizing.wake_at(bigger);
            Some(format!(
                "glave: this computer gets {} when it next wakes.",
                bigger.said(short)
            ))
        }
        Short::Memory => {
            if !person.pod.killed_for_memory().await {
                return None;
            }
            let said = match now.grown(short, ceiling) {
                Some(bigger) => {
                    person.pod.sizing.wake_at(bigger);
                    format!(
                        "glave: this computer ran out of memory at {} and restarts with {}; \
                         anything it ran in the background stopped. Run the command again.",
                        now.said(short),
                        bigger.said(short)
                    )
                }
                None => format!(
                    "glave: this computer ran out of memory at {}, as much as it gets.",
                    now.said(short)
                ),
            };
            let awake = match &*person.state.borrow() {
                State::Awake(home) => Some(home.clone()),
                _ => None,
            };
            if let Some(home) = awake {
                slept(person, &home).await;
            }
            Some(said)
        }
    }
}

/// Lets go of `person`'s computer, awake with `home` until it went away on its own: what is
/// left of it removed, and counted asleep, so the next call wakes it again.
async fn slept(person: &Person, home: &WorkspaceInfo) {
    // Only under its lease: a Pod by its name may be another server's by now.
    if holds(person) {
        let _ = person.computers.destroy(home.computer).await;
    }
    person.state.send_if_modified(|state| match state {
        State::Awake(awake) if awake.computer == home.computer => {
            *state = State::Asleep;
            true
        }
        _ => false,
    });
}

/// Keeps `held`, the lease of `person`'s computer, renewed while it wakes or is awake, in place
/// of any renewal before it.
fn renew_while_awake(person: &Arc<Person>, held: lease::Held) {
    *person.held.lock().unwrap_or_else(PoisonError::into_inner) = Some(held);
    let task = tokio::spawn(renewing(
        Arc::downgrade(person),
        person.leasing.clone(),
        held,
    ));
    let mut renewing = person
        .renewing
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(before) = renewing.replace(task) {
        before.abort();
    }
}

/// Renews `held` every [`lease::RENEW`] while `person`'s computer wakes or is awake, and once a
/// minute looks that it still runs: what is left of one that slept on its own is removed under
/// the lease. Once it sleeps, renewing stops and the lease lapses; once it is no longer surely
/// this server's, the wake stops and the computer is left to whoever holds it.
async fn renewing(person: std::sync::Weak<Person>, leasing: Leasing, mut held: lease::Held) {
    let look = (HEARTBEAT.as_secs() / lease::RENEW.as_secs()).max(1);
    let mut renewed = 0u64;
    loop {
        tokio::time::sleep(lease::RENEW).await;
        let Some(person) = person.upgrade() else {
            return;
        };
        let now = person.state.borrow().clone();
        if matches!(now, State::Asleep | State::Failed(..)) {
            return;
        }
        if !leasing.renew(&mut held).await {
            lost(&person);
            return;
        }
        *person.held.lock().unwrap_or_else(PoisonError::into_inner) = Some(held);
        renewed = renewed.saturating_add(1);
        if let State::Awake(home) = now
            && renewed.is_multiple_of(look)
            && matches!(person.pod.running().await, Ok(false))
        {
            slept(&person, &home).await;
        }
    }
}

/// Leaves `person`'s computer to the server that holds its lease now: a wake running here
/// stops, and it counts asleep here, its Pod touched no more.
fn lost(person: &Person) {
    *person.held.lock().unwrap_or_else(PoisonError::into_inner) = None;
    if let Some(task) = person
        .waking
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
    {
        task.abort();
    }
    person.state.send_replace(State::Asleep);
}

/// Whether this server surely holds `person`'s computer's lease, so it may make, change or
/// remove its Pod.
fn holds(person: &Person) -> bool {
    person
        .held
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_some_and(|held| held.holds())
}

/// Starts waking `person`'s computer from `environment`, unless it is awake or waking.
fn start(person: &Arc<Person>, environment: &Environment) {
    let starts = person.state.send_if_modified(|state| match state {
        State::Asleep | State::Failed(..) => {
            *state = State::Waking;
            true
        }
        State::Waking | State::Awake(_) => false,
    });
    if starts {
        let task = tokio::spawn(woken(Arc::clone(person), environment.clone()));
        let mut waking = person.waking.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(before) = waking.replace(task) {
            before.abort();
        }
    }
}

/// `person`'s computer awake, woken from `environment` if it sleeps, and the workspace of
/// their home on it: once awake, at once. One that ended on its own is let go of and woken
/// again.
async fn awake(person: &Arc<Person>, environment: &Environment) -> Result<Home, ComputerError> {
    let was = person.state.borrow().clone();
    if let State::Awake(home) = was
        && !gone(person, &home).await
    {
        return Ok(Home {
            computers: Arc::clone(&person.computers) as Arc<dyn Computers>,
            home,
        });
    }
    if let State::Failed(at, why) = &*person.state.borrow()
        && at.elapsed() < RETRY_AFTER
    {
        return Err(ComputerError::Failed(format!(
            "your computer could not wake: {why}"
        )));
    }
    // Only the server holding its lease wakes it, so none replaces a Pod another drives.
    let held = person.leasing.take(person.org, person.id).await?;
    renew_while_awake(person, held);
    start(person, environment);
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

/// Whether `person`'s computer, awake with `home`, has ended on its own; if so, what is left of
/// it is removed and it counts asleep, so the next ask wakes it again. One that cannot be
/// reached but whose Pod still runs, or whose Pod cannot be read (the API server restarting),
/// is kept: waking would replace it, and its next call tries a new bridge.
async fn gone(person: &Person, home: &WorkspaceInfo) -> bool {
    match person.computers.status(home.computer).await {
        Ok(info) if info.state == ComputerState::Running => false,
        _ if !matches!(person.pod.running().await, Ok(false)) => false,
        _ => {
            // Killed whole for memory with no command running: its next wake is bigger.
            if person.pod.ran_out_of_memory().await
                && let Some(bigger) = person.pod.sizing.now().grown(Short::Memory, person.ceiling)
            {
                person.pod.sizing.wake_at(bigger);
            }
            slept(person, home).await;
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
            let mut renewing = person
                .renewing
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(task) = renewing.take() {
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
