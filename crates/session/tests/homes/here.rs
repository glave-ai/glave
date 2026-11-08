//! People's computers on this machine, standing in for the cluster's: each person's a folder
//! of their own, planned until a call needs it, then woken, every wake counted.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use glave_computer::{Largest, LocalComputers, Peek, Wake, Waking};
use glave_event::PrincipalId;
use glave_harness::Homes;
use glave_ports::OrgId;
use glave_ports::computer::{
    ComputerError, ComputerId, Computers, DiskSpace, Home, Sandbox, Sources, WorkspaceId,
    WorkspaceInfo,
};

/// People's computers on this machine: each person's a folder of their own with its scratch
/// folders, planned until a call needs it, then woken; every wake is counted.
pub(crate) struct Here {
    pub(crate) base: PathBuf,
    /// How many times a call asked for a computer, woken or not.
    pub(crate) asked: Arc<AtomicUsize>,
    /// How many times a computer was woken.
    pub(crate) woken: Arc<AtomicUsize>,
    /// How many turns and tasks hold a computer now.
    pub(crate) holds: Arc<AtomicUsize>,
    /// How many held a computer each time one was woken.
    pub(crate) held_when_woken: Arc<Mutex<Vec<usize>>>,
    /// While what this names is there, each computer's disk says it is full.
    pub(crate) full_while: Arc<Mutex<Option<PathBuf>>>,
    /// What each computer says uses its disk.
    pub(crate) largest: Largest,
    pub(crate) planned: Mutex<BTreeMap<PrincipalId, Home>>,
    /// The sandbox commands run in there; none, its wall the only one, unless a test sets it.
    pub(crate) sandbox: Option<Sandbox>,
    /// What starts a terminal's shell there, in a session of its own; none opens without one.
    pub(crate) helper: Option<PathBuf>,
}

/// One turn or task holding a computer, counted while it is kept.
struct Hold(Arc<AtomicUsize>);

impl Hold {
    fn new(holds: &Arc<AtomicUsize>) -> Self {
        holds.fetch_add(1, Ordering::SeqCst);
        Self(Arc::clone(holds))
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Here {
    pub(crate) fn new(base: &Path) -> Self {
        std::fs::create_dir_all(base).unwrap();
        Self {
            base: base.canonicalize().unwrap(),
            asked: Arc::default(),
            woken: Arc::default(),
            holds: Arc::default(),
            held_when_woken: Arc::default(),
            full_while: Arc::default(),
            largest: Largest::default(),
            planned: Mutex::new(BTreeMap::new()),
            sandbox: None,
            helper: None,
        }
    }

    pub(crate) fn woken(&self) -> usize {
        self.woken.load(Ordering::SeqCst)
    }

    pub(crate) fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }

    /// Where `person`'s computer keeps its scratch folders.
    pub(crate) fn scratch(&self, person: PrincipalId) -> PathBuf {
        self.base.join(person.to_string()).join("scratch")
    }

    /// Where `person`'s computer makes new projects' folders.
    pub(crate) fn projects(&self, person: PrincipalId) -> PathBuf {
        self.home_folder(person).join("glave")
    }

    /// `person`'s home on their computer.
    pub(crate) fn home_folder(&self, person: PrincipalId) -> PathBuf {
        self.base.join(person.to_string()).join("home")
    }

    /// `person`'s computer, planned: woken, the first time a call needs it, as a computer of
    /// this machine with its home in a folder of its own.
    fn plan(&self, person: PrincipalId) -> Home {
        let folder = self.home_folder(person);
        std::fs::create_dir_all(&folder).unwrap();
        let (scratch, projects) = (self.scratch(person), self.projects(person));
        let helper = self.helper.clone();
        let (asked, woken, holds, held) = (
            Arc::clone(&self.asked),
            Arc::clone(&self.woken),
            Arc::clone(&self.holds),
            Arc::clone(&self.held_when_woken),
        );
        let awake: Arc<tokio::sync::Mutex<Option<Home>>> = Arc::default();
        let at = folder.clone();
        let full_while = Arc::clone(&self.full_while);
        let largest = self.largest.clone();
        let peek = found_in(&awake);
        let wake: Wake = Arc::new(move || {
            let full_while = Arc::clone(&full_while);
            let largest = largest.clone();
            asked.fetch_add(1, Ordering::SeqCst);
            let (woken, holds, held, awake) = (
                Arc::clone(&woken),
                Arc::clone(&holds),
                Arc::clone(&held),
                Arc::clone(&awake),
            );
            let (scratch, projects, at) = (scratch.clone(), projects.clone(), at.clone());
            let helper = helper.clone();
            let full_while = Arc::clone(&full_while);
            Box::pin(async move {
                let mut awake = awake.lock().await;
                if let Some(home) = awake.as_ref() {
                    return Ok(home.clone());
                }
                woken.fetch_add(1, Ordering::SeqCst);
                held.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(holds.load(Ordering::SeqCst));
                let full_while = Arc::clone(&full_while);
                // Its home is its own: nothing here looks at this machine's person's.
                let environment = std::env::vars_os()
                    .filter(|(name, _)| name != "HOME")
                    .chain([("HOME".into(), at.clone().into_os_string())]);
                let computers = LocalComputers::for_here(environment)
                    .with_disk_measure(move || {
                        let full = full_while
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .as_ref()
                            .is_some_and(|path: &PathBuf| path.exists());
                        DiskSpace {
                            size: 100,
                            free: if full { 9 } else { 50 },
                        }
                    })
                    .with_largest(largest.clone())
                    .with_worktrees(at.join(".glave/worktrees"))
                    .with_scratch(scratch)
                    .with_projects(projects)
                    .known_as(ComputerId::from_uuid(person.as_uuid()));
                let computers = started_by(computers, helper.as_deref());
                let (computers, info) = computers.open_here(&at).await?;
                let home = Home {
                    computers: computers as Arc<dyn Computers>,
                    home: info,
                };
                *awake = Some(home.clone());
                Ok(home)
            })
        });
        let path = folder.to_string_lossy().into_owned();
        let home = WorkspaceInfo {
            id: WorkspaceId::from_uuid(person.as_uuid()),
            computer: ComputerId::from_uuid(person.as_uuid()),
            root: path.clone(),
            sources: Sources::Folder { path },
            home: None,
            temp: None,
            at: None,
        };
        let scratch = self.scratch(person);
        Waking::home((wake, peek), None, home, &scratch.to_string_lossy())
    }
}

/// `computers`, its terminals' shells started by `helper` where there is one.
fn started_by(computers: LocalComputers, helper: Option<&Path>) -> LocalComputers {
    match helper {
        Some(helper) => computers.with_helper(helper),
        None => computers,
    }
}

/// A computer found awake, without waking it, as `awake` holds it once woken.
fn found_in(awake: &Arc<tokio::sync::Mutex<Option<Home>>>) -> Peek {
    let awake = Arc::clone(awake);
    Arc::new(move || {
        let awake = Arc::clone(&awake);
        Box::pin(async move { Ok(awake.lock().await.clone()) })
    })
}

impl Homes for Here {
    fn hold(&self, _org: OrgId, _person: PrincipalId) -> Result<Box<dyn Send>, ComputerError> {
        Ok(Box::new(Hold::new(&self.holds)))
    }

    fn sandbox(&self) -> Option<Sandbox> {
        self.sandbox.clone()
    }

    fn home(&self, _org: OrgId, person: PrincipalId) -> Result<Home, ComputerError> {
        let mut planned = self.planned.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(planned
            .entry(person)
            .or_insert_with(|| self.plan(person))
            .clone())
    }
}
