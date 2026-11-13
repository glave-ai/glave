//! The people of a deployment and their computers on one cluster (`specs/computers.md`, "A
//! person's computer in the cloud"): each person's computer handed to whatever works there as
//! planned, asleep or awake, and woken only by the first call that needs it (decision 6), once
//! however many ask at the same time; kept awake while a turn or task holds it. A computer
//! sleeps on its own once nothing has asked it anything for 2 hours and no command runs
//! there in the background (`computer serve --idle`); the next ask finds it ended and wakes it
//! again.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use glave_event::PrincipalId;
use glave_ports::OrgId;
use glave_ports::computer::{
    ComputerError, ComputerId, Computers, Environment, Home, Image, Limits, Sandbox, SandboxLevel,
    SandboxNetwork, Sources, State as ComputerState, WorkspaceId, WorkspaceInfo,
};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::Kubernetes;
use super::home::PersonComputer;
use super::objects::{IDLE_MINUTES, PERSON_HOME, PERSON_SCRATCH};
use super::served::Seen;
use super::sizing::{CEILING, Size};
use crate::worker::WorkerComputers;
use crate::worker::limits::pinned;
use crate::{Grow, Lost, Peek, Short, Wake, Waking};

mod grow;
mod lease;
mod reach;

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

/// Where one person's computer is. Each wake this server starts is numbered after the last, so
/// what one wake leaves running, the renewal of its lease, never acts on a later one.
#[derive(Debug, Clone)]
enum State {
    Asleep,
    /// Being woken by the wake of that number.
    Waking(u64),
    /// Awake from the wake of that number: the workspace of its home.
    Awake(u64, WorkspaceInfo),
    /// The last wake failed, when, and why; a call a minute later or more tries again.
    Failed(Instant, String),
}

impl State {
    /// Whether it is no longer waking: awake, or its wake failed.
    fn settled(&self) -> bool {
        !matches!(self, Self::Waking(_))
    }

    /// The wake that is waking it or woke it; none while it sleeps or once its wake failed.
    fn wake(&self) -> Option<u64> {
        match self {
            Self::Waking(wake) | Self::Awake(wake, _) => Some(*wake),
            Self::Asleep | Self::Failed(..) => None,
        }
    }
}

/// One person's computer, as planned and as it is, and the wake running for it, if one is.
struct Person {
    /// Whose it is, as its lease names it.
    org: OrgId,
    id: PrincipalId,
    leasing: Leasing,
    /// Its lease while this server holds it, with the wake it was taken for, and what renews it.
    held: Mutex<Option<(u64, lease::Held)>>,
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
    /// How many wakes this server has started for it: the next one's number.
    wakes: AtomicU64,
    waking: Mutex<Option<JoinHandle<()>>>,
    /// Held by the one sleep at a time, until it has let go of the lease.
    sleeping: tokio::sync::Mutex<()>,
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
    /// Where each computer's lease is kept, so one server at a time makes or removes its Pod.
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
    /// People's computers on `cluster`, each running `image` (pinned by digest) within
    /// `limits`, with the network unless they take it, with a disk of `disk_gb`, each made,
    /// grown or removed by this server only while it holds the computer's lease (`leasing`).
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
        let environment = environment(image);
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
            limits: limits.clone(),
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
        let computers = WorkerComputers::new(computer.clone(), self.limits.clone());
        let home = planned_home(key)?;
        let ceiling = self.ceiling;
        let person = Arc::new_cyclic(|me: &std::sync::Weak<Person>| {
            let grows = std::sync::Weak::clone(me);
            let grow: Grow = Arc::new(move |short| {
                let me = std::sync::Weak::clone(&grows);
                Box::pin(async move { grow::grow(&me.upgrade()?, short, ceiling).await })
            });
            let looks = std::sync::Weak::clone(me);
            let lost: Lost = Arc::new(move || {
                let me = std::sync::Weak::clone(&looks);
                Box::pin(async move {
                    match me.upgrade() {
                        Some(person) => person.pod.ended().await,
                        None => false,
                    }
                })
            });
            let peeks = std::sync::Weak::clone(me);
            let peek: Peek = Arc::new(move || {
                let me = std::sync::Weak::clone(&peeks);
                Box::pin(async move {
                    match me.upgrade() {
                        Some(person) => peeked(&person).await,
                        None => Ok(None),
                    }
                })
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
                planned: Waking::home((wake, peek), Some((grow, lost)), home, PERSON_SCRATCH),
                pod: computer,
                ceiling,
                growing: tokio::sync::Mutex::new(()),
                computers: Arc::new(computers),
                state: watch::Sender::new(State::Asleep),
                wakes: AtomicU64::new(0),
                waking: Mutex::new(None),
                sleeping: tokio::sync::Mutex::new(()),
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
    /// minute, and its lease taken when no server holds it.
    pub fn hold(&self, org: OrgId, person: PrincipalId) -> Result<Holding, ComputerError> {
        let person = self.person(org, person)?;
        let every = HEARTBEAT.min(Duration::from_mins(u64::from(self.idle_minutes)) / 4);
        Ok(Holding(tokio::spawn(async move {
            loop {
                tokio::time::sleep(every).await;
                let awake = match &*person.state.borrow() {
                    State::Awake(wake, home) => Some((*wake, home.computer)),
                    _ => None,
                };
                if let Some((wake, computer)) = awake {
                    let _ = person.computers.status(computer).await;
                    // Using it, so this server keeps its lease when no other does: a computer
                    // in use always has a holder to look after it (decision 18).
                    if !holds(&person)
                        && let Ok(Some(held)) = person.leasing.take(person.org, person.id).await
                    {
                        renew_while_awake(&person, held, wake);
                    }
                }
            }
        })))
    }

    /// The computer of `person` in `org` and the workspace of their home on it, as planned: at
    /// once, asleep or awake. The first call on it that needs the computer wakes it, if it
    /// sleeps, and waits for that wake, which every other call asking meanwhile shares; one that
    /// ended on its own (idle, or its node went) is woken again. Awake through another server, it
    /// is reached as it is, through a connection of this server's own.
    pub fn home(&self, org: OrgId, person: PrincipalId) -> Result<Home, ComputerError> {
        Ok(self.person(org, person)?.planned.clone())
    }
}

/// What every person's computer is woken from: `image`, asking nothing of the network, so it
/// has the network unless policy takes it, and then wakes without it rather than refused.
fn environment(image: &str) -> Environment {
    Environment {
        image: Image::Oci(image.to_owned()),
        ..Environment::host()
    }
}

/// The sandbox commands on a person's computer run in, at its widest (each mode narrows it),
/// with the network as `network` says: its home's credentials and glave's folder there hidden.
#[must_use]
pub fn person_sandbox(network: bool) -> Sandbox {
    Sandbox {
        level: SandboxLevel::Project,
        network: SandboxNetwork::when(network),
        home: Some(PERSON_HOME.into()),
        glave_home: Some(format!("{PERSON_HOME}/.glave").into()),
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
        at: None,
    })
}

/// Lets go of `person`'s computer, awake with `home` until it went away on its own: what is
/// left of it removed and its lease let go of, and only then counted asleep, so the next call
/// wakes it again and that wake takes the lease after this sleep let go of it. One sleep at a
/// time: a call that finds the computer gone meanwhile waits for this one, then wakes it. One
/// no longer awake with `home`, asleep already or woken again since, is not this sleep's.
async fn slept(person: &Person, home: &WorkspaceInfo) {
    let _sleeping = person.sleeping.lock().await;
    let wake = match &*person.state.borrow() {
        State::Awake(wake, awake) if awake.computer == home.computer => *wake,
        _ => return,
    };
    // Only under its lease: a Pod by its name may be another server's by now. Let go of once
    // it is removed, so another server wakes it at once rather than once the lease lapses.
    let held = *person.held.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((_, held)) = held.filter(|(of, held)| *of == wake && held.holds()) {
        let _ = person.computers.destroy(home.computer).await;
        person.leasing.release(&held).await;
    }
    let_go_of_lease(person, wake);
    let_go(person, home);
}

/// Forgets the lease of `person`'s computer that its wake `wake` took, if this server still
/// keeps it; a later wake's is kept.
fn let_go_of_lease(person: &Person, wake: u64) {
    let mut held = person.held.lock().unwrap_or_else(PoisonError::into_inner);
    if held.is_some_and(|(of, _)| of == wake) {
        *held = None;
    }
}

/// Counts `person`'s computer asleep here, if it was awake with `home`: the next call reaches
/// it again, or wakes it.
fn let_go(person: &Person, home: &WorkspaceInfo) {
    person.computers.forget(home.computer);
    person.state.send_if_modified(|state| match state {
        State::Awake(_, awake) if awake.computer == home.computer => {
            *state = State::Asleep;
            true
        }
        _ => false,
    });
}

/// Keeps `held`, the lease of `person`'s computer, renewed while its wake `wake` goes on, in
/// place of any renewal before it; a wake that has ended keeps nothing.
fn renew_while_awake(person: &Arc<Person>, held: lease::Held, wake: u64) {
    if person.state.borrow().wake() != Some(wake) {
        return;
    }
    *person.held.lock().unwrap_or_else(PoisonError::into_inner) = Some((wake, held));
    let task = tokio::spawn(renewing(
        Arc::downgrade(person),
        person.leasing.clone(),
        held,
        wake,
    ));
    let mut renewing = person
        .renewing
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(before) = renewing.replace(task) {
        before.abort();
    }
}

/// Renews `held` every [`lease::RENEW`] while `person`'s computer wakes or is awake from its
/// wake `wake`, and once a minute looks that it still runs: what is left of one that slept on
/// its own is removed under the lease, and let go of. Once that wake ends (the computer sleeps,
/// or its wake failed), renewing stops, a later wake left to its own; once the lease is no
/// longer surely this server's, the wake stops and the computer is left to whoever holds it.
async fn renewing(
    person: std::sync::Weak<Person>,
    leasing: Leasing,
    mut held: lease::Held,
    wake: u64,
) {
    let look = (HEARTBEAT.as_secs() / lease::RENEW.as_secs()).max(1);
    let mut renewed = 0u64;
    loop {
        tokio::time::sleep(lease::RENEW).await;
        let Some(person) = person.upgrade() else {
            return;
        };
        let now = person.state.borrow().clone();
        if now.wake() != Some(wake) {
            return;
        }
        if !leasing.renew(&mut held).await {
            lost(&person, wake);
            return;
        }
        if let Some((of, kept)) = person
            .held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
            && *of == wake
        {
            *kept = held;
        }
        renewed = renewed.saturating_add(1);
        if let State::Awake(_, home) = now
            && renewed.is_multiple_of(look)
            && matches!(person.pod.running().await, Ok(false))
        {
            slept(&person, &home).await;
        }
    }
}

/// Leaves `person`'s computer to the server that holds its lease now, while its wake `wake`
/// goes on: that wake stops if it is still waking, and the computer counts asleep here, its Pod
/// touched no more; the next call reaches it again. A later wake is not this one's to stop.
fn lost(person: &Person, wake: u64) {
    let_go_of_lease(person, wake);
    let mut was = None;
    person.state.send_if_modified(|state| {
        if state.wake() != Some(wake) {
            return false;
        }
        if let Some(task) = person
            .waking
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            task.abort();
        }
        was = Some(std::mem::replace(state, State::Asleep));
        true
    });
    if let Some(State::Awake(_, home)) = was {
        person.computers.forget(home.computer);
    }
}

/// Whether this server surely holds `person`'s computer's lease, so it may make, change or
/// remove its Pod.
fn holds(person: &Person) -> bool {
    person
        .held
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_some_and(|(_, held)| held.holds())
}

/// Starts waking `person`'s computer, unless it is awake or waking, as the next of its wakes:
/// `reach`, given that wake's number, reaches it and opens their home on it.
fn start<F>(person: &Arc<Person>, reach: impl FnOnce(u64) -> F)
where
    F: Future<Output = Result<WorkspaceInfo, ComputerError>> + Send + 'static,
{
    let mut started = None;
    person.state.send_if_modified(|state| match state {
        State::Asleep | State::Failed(..) => {
            let wake = person.wakes.fetch_add(1, Ordering::Relaxed);
            *state = State::Waking(wake);
            started = Some(wake);
            true
        }
        State::Waking(_) | State::Awake(..) => false,
    });
    if let Some(wake) = started {
        let task = tokio::spawn(woken(Arc::clone(person), wake, reach(wake)));
        let mut waking = person.waking.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(before) = waking.replace(task) {
            before.abort();
        }
    }
}

/// `person`'s computer awake, reached or woken from `environment` ([`reached`]), and the
/// workspace of their home on it: once awake, at once. One that ended on its own is let go of
/// and woken again.
async fn awake(person: &Arc<Person>, environment: &Environment) -> Result<Home, ComputerError> {
    let was = person.state.borrow().clone();
    if let State::Awake(_, home) = was
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
    start(person, |wake| {
        reached(Arc::clone(person), environment.clone(), wake)
    });
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
        State::Awake(_, home) => Ok(Home {
            computers: Arc::clone(&person.computers) as Arc<dyn Computers>,
            home,
        }),
        State::Failed(_, why) => Err(ComputerError::Failed(format!(
            "your computer could not wake: {why}"
        ))),
        State::Asleep | State::Waking(_) => Err(ComputerError::Failed(
            "your computer went to sleep as it woke".into(),
        )),
    }
}

/// `person`'s computer as it is now, never woken: awake with its home, as this server drives
/// it; none while it sleeps or is being woken (a wake starts it anew); unreachable while its
/// Pod runs, or can't be read, without this server driving it: another server's, or one this
/// server woke before it restarted.
async fn peeked(person: &Person) -> Result<Option<Home>, ComputerError> {
    let now = person.state.borrow().clone();
    match now {
        State::Awake(_, home) => Ok(Some(Home {
            computers: Arc::clone(&person.computers) as Arc<dyn Computers>,
            home,
        })),
        State::Waking(_) => Ok(None),
        State::Asleep | State::Failed(..) => match person.pod.running().await {
            Ok(false) => Ok(None),
            Ok(true) | Err(_) => Err(ComputerError::Unreachable(
                "your computer runs, but not through this server, which so cannot reach it".into(),
            )),
        },
    }
}

/// Whether `person`'s computer, awake with `home`, has ended on its own; if so, what is left of
/// it is removed and it counts asleep, so the next ask wakes it again. One that cannot be
/// reached but whose Pod still runs, or whose Pod cannot be read (the API server restarting),
/// is kept: waking would replace it, and its next call tries a new bridge. One whose Pod
/// another server woke again since is let go of here, the new Pod left as it is, and reached
/// at the next ask.
async fn gone(person: &Person, home: &WorkspaceInfo) -> bool {
    if matches!(
        person.computers.status(home.computer).await,
        Ok(info) if info.state == ComputerState::Running
    ) {
        return false;
    }
    let reached_in = person.computers.pod_of(home.computer);
    match person.pod.running_as().await {
        Err(_) => false,
        Ok(Some(pod)) if reached_in.as_ref().is_none_or(|reached| *reached == pod) => false,
        Ok(Some(_)) => {
            let_go(person, home);
            true
        }
        Ok(None) => {
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

/// Reaches `person`'s computer as its wake `wake` (`reaching`), then says how it went to whoever
/// waits, while that wake still goes on.
async fn woken(
    person: Arc<Person>,
    wake: u64,
    reaching: impl Future<Output = Result<WorkspaceInfo, ComputerError>>,
) {
    let state = match reaching.await {
        Ok(home) => State::Awake(wake, home),
        Err(error) => State::Failed(Instant::now(), error.message().to_owned()),
    };
    person.state.send_if_modified(|now| {
        let ours = matches!(now, State::Waking(of) if *of == wake);
        if ours {
            *now = state;
        }
        ours
    });
}

/// `person`'s computer reached from `environment` ([`reach::reached`]) by its wake `wake`, and
/// their home opened on it.
async fn reached(
    person: Arc<Person>,
    environment: Environment,
    wake: u64,
) -> Result<WorkspaceInfo, ComputerError> {
    let machine = Reaching {
        person: &person,
        environment: &environment,
        wake,
    };
    reach::reached(
        &machine,
        &person.leasing,
        (person.org, person.id),
        WAKE_WAIT,
    )
    .await
}

/// A person's computer as this server reaches it by its wake `wake`, woken from `environment`.
struct Reaching<'a> {
    person: &'a Arc<Person>,
    environment: &'a Environment,
    wake: u64,
}

impl reach::Machine for Reaching<'_> {
    async fn seen(&self) -> Result<Seen, ComputerError> {
        self.person.pod.seen().await
    }

    async fn attach(&self, pod: &str) -> Result<WorkspaceInfo, ComputerError> {
        let computer = self.person.computers.attach(self.environment, pod).await?;
        home_on(self.person, computer.id).await
    }

    async fn wake(&self) -> Result<WorkspaceInfo, ComputerError> {
        let computer = self.person.computers.create(self.environment).await?;
        home_on(self.person, computer.id).await
    }

    fn keep(&self, held: lease::Held) {
        renew_while_awake(self.person, held, self.wake);
    }
}

/// The person's home opened on `computer`, just reached; a computer whose home won't open, or
/// whose opening was stopped, is let go of here.
async fn home_on(person: &Person, computer: ComputerId) -> Result<WorkspaceInfo, ComputerError> {
    let mut unopened = Unopened {
        computers: &person.computers,
        computer,
        opened: false,
    };
    let home = Sources::Folder {
        path: PERSON_HOME.into(),
    };
    let opened = person.computers.open(computer, &home).await?;
    unopened.opened = true;
    Ok(opened)
}

/// A computer reached whose home is not open yet: let go of if it never is.
struct Unopened<'a> {
    computers: &'a WorkerComputers,
    computer: ComputerId,
    opened: bool,
}

impl Drop for Unopened<'_> {
    fn drop(&mut self) {
        if !self.opened {
            self.computers.forget(self.computer);
        }
    }
}

#[cfg(test)]
mod sleeps;
#[cfg(test)]
mod tests;
