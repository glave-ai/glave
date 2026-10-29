//! Each person's own computer, where a deployment gives people one (`specs/computers.md`, "A
//! person's computer in the cloud"): opening the app starts waking it, and their threads work
//! there, in a scratch folder on it, recorded as on their own computer, and opened there again
//! by a resumed thread's turn, never by reading it. A computer of this machine stands in for
//! the cluster's.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use glave_computer::LocalComputers;
use glave_event::{ComputerKind, PrincipalId};
use glave_harness::Homes;
use glave_ports::computer::{ComputerError, ComputerId, Computers, Home};
use glave_ports::{BoxFuture, OrgId};
use glave_session::Answering;
use glave_session::protocol::{Reply, Request, Show};

use crate::support::*;

/// People's computers on this machine: each person's a folder of their own with its scratch
/// folders, woken the first time it is asked for; every wake asked for is kept.
struct Here {
    base: PathBuf,
    woken: Mutex<Vec<(OrgId, PrincipalId)>>,
    /// How many times a computer was asked for, to work on it.
    asked: AtomicUsize,
    /// How many turns and tasks hold a computer now.
    holds: Arc<AtomicUsize>,
    /// How many held a computer each time one was asked for.
    held_when_asked: Mutex<Vec<usize>>,
    awake: tokio::sync::Mutex<BTreeMap<PrincipalId, Home>>,
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
    fn new(base: &Path) -> Self {
        Self {
            base: base.to_owned(),
            woken: Mutex::new(Vec::new()),
            asked: AtomicUsize::new(0),
            holds: Arc::default(),
            held_when_asked: Mutex::new(Vec::new()),
            awake: tokio::sync::Mutex::new(BTreeMap::new()),
        }
    }

    fn woken(&self) -> Vec<(OrgId, PrincipalId)> {
        self.woken.lock().unwrap().clone()
    }

    /// Where `person`'s computer keeps its scratch folders.
    fn scratch(&self, person: PrincipalId) -> PathBuf {
        self.base.join(person.to_string()).join("scratch")
    }
}

impl Homes for Here {
    fn wake(&self, org: OrgId, person: PrincipalId) -> Result<(), ComputerError> {
        self.woken
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((org, person));
        Ok(())
    }

    fn hold(&self, _org: OrgId, _person: PrincipalId) -> Result<Box<dyn Send>, ComputerError> {
        Ok(Box::new(Hold::new(&self.holds)))
    }

    fn home(&self, _org: OrgId, person: PrincipalId) -> BoxFuture<'_, Result<Home, ComputerError>> {
        Box::pin(async move {
            self.asked.fetch_add(1, Ordering::SeqCst);
            self.held_when_asked
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(self.holds.load(Ordering::SeqCst));
            let mut awake = self.awake.lock().await;
            if let Some(home) = awake.get(&person) {
                return Ok(home.clone());
            }
            let folder = self.base.join(person.to_string()).join("home");
            std::fs::create_dir_all(&folder).unwrap();
            let computers = LocalComputers::for_here(std::env::vars_os())
                .with_scratch(self.scratch(person))
                .known_as(ComputerId::from_uuid(person.as_uuid()));
            let (computers, info) = computers.open_here(&folder).await?;
            let home = Home {
                computers: computers as Arc<dyn Computers>,
                home: info,
            };
            awake.insert(person, home.clone());
            Ok(home)
        })
    }
}

#[test]
fn a_person_s_threads_work_on_their_own_computer_woken_as_they_open_the_app() {
    let dir = folder("session-homes");
    block_on!(async {
        let homes = Arc::new(Here::new(&dir.join("people")));
        let log = Shared::default();
        let mut setup = Setup::asking(Vec::new(), Answering::NoOne);
        setup.homes = Some(Arc::clone(&homes) as Arc<dyn Homes>);
        let (server, _) = setup.build_on(&dir, log.clone()).await;
        let person = server.person().unwrap();

        // The app opens: its threads listed, the computer's wake started.
        server
            .request(Request::ThreadList {
                project: None,
                limit: None,
                show: Some(Show::Active),
                cursor: None,
            })
            .await
            .unwrap();
        assert_eq!(homes.woken(), vec![(scope().org(), person)]);

        // A chat of their own: its scratch folder on their computer, not the server's.
        let started = server
            .request(Request::ThreadStart {
                project: None,
                worktree: None,
                background: false,
                computer: None,
                model: None,
                permissions: None,
                effort: None,
            })
            .await
            .unwrap();
        let Reply::Thread { thread } = started else {
            panic!("a thread, not {started:?}")
        };
        let workspace = thread.workspace.clone().unwrap();
        assert_eq!(workspace.computer.kind, ComputerKind::Person);
        assert_eq!(workspace.computer.id, person.as_uuid());
        let scratch = homes.scratch(person);
        assert!(
            Path::new(&workspace.root).starts_with(&scratch),
            "{} is on their computer, under {}",
            workspace.root,
            scratch.display()
        );

        // A server started later on the same store resumes it on their computer: its scratch
        // folder, removed meanwhile, made again there.
        std::fs::remove_dir_all(&workspace.root).unwrap();
        let mut next = Setup::asking(vec![answer(Some("Hi."), None)], Answering::NoOne);
        next.seed = 2;
        next.settings.person = Some(person);
        next.projects = Some(Arc::clone(server.projects()));
        next.homes = Some(Arc::clone(&homes) as Arc<dyn Homes>);
        let (again, _) = next.build_on(&dir, log).await;
        // Reading it waits for no computer: only a turn needs one.
        let asked = homes.asked.load(Ordering::SeqCst);
        again
            .request(Request::ThreadGet { thread: thread.id })
            .await
            .unwrap();
        assert_eq!(
            homes.asked.load(Ordering::SeqCst),
            asked,
            "read without waking"
        );
        turn(&again, thread.id, "hello").await.unwrap();
        let held = homes.held_when_asked.lock().unwrap().clone();
        assert_eq!(held.last(), Some(&1), "the turn kept it awake: {held:?}");
        assert!(
            Path::new(&workspace.root).is_dir(),
            "opened again on their computer"
        );
    });
}
