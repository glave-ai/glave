//! Each person's own computer, where a deployment gives people one (`specs/computers.md`, "A
//! person's computer in the cloud"): their threads work there, in a scratch folder on it,
//! recorded as on their own computer; opening the app, starting a chat or a thread in a project
//! there, reading it and a turn that only talks never wake it, and the first tool call that
//! needs it does, once, while the turn holds it. A computer of this machine stands in for the
//! cluster's.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use glave_computer::{LocalComputers, Wake, Waking};
use glave_event::{ComputerKind, Part, PrincipalId, ThreadId};
use glave_harness::Homes;
use glave_ports::computer::{
    ComputerError, ComputerId, Computers, Home, Sources, WorkspaceId, WorkspaceInfo,
};
use glave_ports::{Message, OrgId, ProjectId};
use glave_session::Answering;
use glave_session::protocol::{NewFolder, Reply, Request, Show, ThreadSummary};
use serde_json::json;

use glave_harness::{Skill, SkillSource, Skills};

use crate::support::*;

/// People's computers on this machine: each person's a folder of their own with its scratch
/// folders, planned until a call needs it, then woken; every wake is counted.
struct Here {
    base: PathBuf,
    /// How many times a call asked for a computer, woken or not.
    asked: Arc<AtomicUsize>,
    /// How many times a computer was woken.
    woken: Arc<AtomicUsize>,
    /// How many turns and tasks hold a computer now.
    holds: Arc<AtomicUsize>,
    /// How many held a computer each time one was woken.
    held_when_woken: Arc<Mutex<Vec<usize>>>,
    planned: Mutex<BTreeMap<PrincipalId, Home>>,
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
        std::fs::create_dir_all(base).unwrap();
        Self {
            base: base.canonicalize().unwrap(),
            asked: Arc::default(),
            woken: Arc::default(),
            holds: Arc::default(),
            held_when_woken: Arc::default(),
            planned: Mutex::new(BTreeMap::new()),
        }
    }

    fn woken(&self) -> usize {
        self.woken.load(Ordering::SeqCst)
    }

    fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }

    /// Where `person`'s computer keeps its scratch folders.
    fn scratch(&self, person: PrincipalId) -> PathBuf {
        self.base.join(person.to_string()).join("scratch")
    }

    /// Where `person`'s computer makes new projects' folders.
    fn projects(&self, person: PrincipalId) -> PathBuf {
        self.home_folder(person).join("glave")
    }

    /// `person`'s home on their computer.
    fn home_folder(&self, person: PrincipalId) -> PathBuf {
        self.base.join(person.to_string()).join("home")
    }

    /// `person`'s computer, planned: woken, the first time a call needs it, as a computer of
    /// this machine with its home in a folder of its own.
    fn plan(&self, person: PrincipalId) -> Home {
        let folder = self.home_folder(person);
        std::fs::create_dir_all(&folder).unwrap();
        let (scratch, projects) = (self.scratch(person), self.projects(person));
        let (asked, woken, holds, held) = (
            Arc::clone(&self.asked),
            Arc::clone(&self.woken),
            Arc::clone(&self.holds),
            Arc::clone(&self.held_when_woken),
        );
        let awake: Arc<tokio::sync::Mutex<Option<Home>>> = Arc::default();
        let at = folder.clone();
        let wake: Wake = Arc::new(move || {
            asked.fetch_add(1, Ordering::SeqCst);
            let (woken, holds, held, awake) = (
                Arc::clone(&woken),
                Arc::clone(&holds),
                Arc::clone(&held),
                Arc::clone(&awake),
            );
            let (scratch, projects, at) = (scratch.clone(), projects.clone(), at.clone());
            Box::pin(async move {
                let mut awake = awake.lock().await;
                if let Some(home) = awake.as_ref() {
                    return Ok(home.clone());
                }
                woken.fetch_add(1, Ordering::SeqCst);
                held.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(holds.load(Ordering::SeqCst));
                let computers = LocalComputers::for_here(std::env::vars_os())
                    .with_worktrees(at.join(".glave/worktrees"))
                    .with_scratch(scratch)
                    .with_projects(projects)
                    .known_as(ComputerId::from_uuid(person.as_uuid()));
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
        };
        Waking::home(wake, None, home, &self.scratch(person).to_string_lossy())
    }
}

impl Homes for Here {
    fn hold(&self, _org: OrgId, _person: PrincipalId) -> Result<Box<dyn Send>, ComputerError> {
        Ok(Box::new(Hold::new(&self.holds)))
    }

    fn home(&self, _org: OrgId, person: PrincipalId) -> Result<Home, ComputerError> {
        let mut planned = self.planned.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(planned
            .entry(person)
            .or_insert_with(|| self.plan(person))
            .clone())
    }
}

/// A thread `server` started for its person, in `project` or a chat of their own.
async fn started(server: &Tested<Shared>, project: Option<ProjectId>) -> ThreadSummary {
    let started = server
        .request(Request::ThreadStart {
            project,
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
    thread
}

#[test]
fn a_person_s_threads_work_on_their_own_computer_woken_only_by_a_tool_call_that_needs_it() {
    let dir = folder("session-homes");
    block_on!(async {
        let homes = Arc::new(Here::new(&dir.join("people")));
        let log = Shared::default();
        let answers = vec![
            answer(Some("Hello."), None),
            answer(None, Some(("read", json!({"path": "notes.txt"})))),
            answer(Some("There are no notes."), None),
        ];
        let mut setup = Setup::asking(answers, Answering::NoOne);
        setup.homes = Some(Arc::clone(&homes) as Arc<dyn Homes>);
        let (server, _) = setup.build_on(&dir, log.clone()).await;
        let person = server.person().unwrap();

        // The app opens: its threads listed, the computer left asleep.
        server
            .request(Request::ThreadList {
                project: None,
                limit: None,
                show: Some(Show::Active),
                cursor: None,
            })
            .await
            .unwrap();
        assert_eq!(homes.woken(), 0, "opening the app wakes nothing");

        // A chat of their own: its scratch folder on their computer, not the server's.
        let thread = started(&server, None).await;
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
        assert_eq!(homes.woken(), 0, "a chat started wakes nothing");

        // A turn that only talks never asks for the computer; one that reads a file wakes it,
        // once, while the turn holds it.
        turn(&server, thread.id, "hi").await.unwrap();
        assert_eq!(homes.asked(), 0, "a turn that only talks never asks for it");
        turn(&server, thread.id, "read my notes").await.unwrap();
        assert_eq!(
            homes.woken(),
            1,
            "the first tool call that needs it wakes it"
        );
        let held = homes.held_when_woken.lock().unwrap().clone();
        assert_eq!(held, vec![1], "the turn holds it as it wakes");
        assert!(
            Path::new(&workspace.root).is_dir(),
            "its scratch folder made there"
        );

        resumed_later(&dir, log, &server, thread.id, &workspace.root).await;
    });
}

/// A server started later on the same store resumes `thread` on its person's computer: reading
/// it and a turn that talks wake nothing, and its scratch folder at `root`, removed
/// meanwhile, is made again there only by the tool call that needs it.
async fn resumed_later(
    dir: &Path,
    log: Shared,
    server: &Tested<Shared>,
    thread: ThreadId,
    root: &str,
) {
    let person = server.person().unwrap();
    std::fs::remove_dir_all(root).unwrap();
    let answers = vec![
        answer(Some("Hi."), None),
        answer(None, Some(("read", json!({"path": "notes.txt"})))),
        answer(Some("Still none."), None),
    ];
    let mut next = Setup::asking(answers, Answering::NoOne);
    next.seed = 2;
    next.settings.person = Some(person);
    next.projects = Some(Arc::clone(server.projects()));
    let later = Arc::new(Here::new(&dir.join("people")));
    next.homes = Some(Arc::clone(&later) as Arc<dyn Homes>);
    let (again, _) = next.build_on(dir, log).await;
    again.request(Request::ThreadGet { thread }).await.unwrap();
    turn(&again, thread, "hello").await.unwrap();
    assert_eq!(later.asked(), 0, "read and talked to without asking for it");
    assert!(!Path::new(root).exists());
    turn(&again, thread, "read my notes again").await.unwrap();
    assert_eq!(later.woken(), 1);
    assert!(Path::new(root).is_dir(), "opened again on their computer");
}

#[test]
fn a_project_s_folder_is_made_on_its_person_s_own_computer_and_its_threads_work_there() {
    let dir = folder("session-homes-project");
    block_on!(async {
        let homes = Arc::new(Here::new(&dir.join("people")));
        let mut setup = Setup::asking(Vec::new(), Answering::NoOne);
        setup.homes = Some(Arc::clone(&homes) as Arc<dyn Homes>);
        let log = Shared::default();
        let (server, _) = setup.build_on(&dir, log.clone()).await;
        let person = server.person().unwrap();
        let create = |folder| Request::ProjectCreate {
            name: "Garden".into(),
            folder: Some(folder),
        };
        let own = homes.home_folder(person).join(".glave");
        std::fs::create_dir_all(&own).unwrap();
        std::fs::write(own.join("AGENTS.md"), "Be brief.\n").unwrap();

        let made = server.request(create(NewFolder::Empty)).await.unwrap();
        let Reply::Project {
            project,
            folder: Some(folder),
        } = made
        else {
            panic!("a project with its folder, not {made:?}")
        };
        assert_eq!(
            folder.checkout.computer,
            person.as_uuid(),
            "checked out on their own computer"
        );
        let root = PathBuf::from(&folder.checkout.path);
        assert!(
            root.starts_with(homes.projects(person)) && root.join(".git").is_dir(),
            "a new repository on their computer: {}",
            root.display()
        );

        // Listed, it is there, and nothing asks their computer to say so.
        let asked = homes.asked();
        let listed = server
            .request(Request::ProjectList { limit: None })
            .await
            .unwrap();
        let Reply::Projects { projects, gone } = listed else {
            panic!("projects, not {listed:?}")
        };
        assert!(projects.iter().any(|listed| listed.id == project.id));
        assert!(gone.is_empty(), "{gone:?}");
        assert_eq!(
            homes.asked(),
            asked,
            "listing asks nothing of their computer"
        );

        started_later(&dir, log, &server, project.id, &root).await;

        // A folder picked by its path is only ever on the server's own computer.
        let picked = server
            .request(create(NewFolder::Path {
                path: dir.to_string_lossy().into_owned(),
            }))
            .await;
        let refused = picked.unwrap_err().to_string();
        assert!(
            refused.contains("a new one or a repository cloned there"),
            "{refused}"
        );
    });
}

/// A server started later on the same store, which has not reached `server`'s person's computer
/// since: a thread in `project` works in its folder at `root` there, opening with what was read
/// there as the folder was made, and starting it and a turn that only talks ask nothing of the
/// computer.
async fn started_later(
    dir: &Path,
    log: Shared,
    server: &Tested<Shared>,
    project: ProjectId,
    root: &Path,
) {
    let person = server.person().unwrap();
    let mut next = Setup::asking(vec![answer(Some("Hello."), None)], Answering::NoOne);
    next.seed = 2;
    next.settings.person = Some(person);
    next.projects = Some(Arc::clone(server.projects()));
    let later = Arc::new(Here::new(&dir.join("people")));
    next.homes = Some(Arc::clone(&later) as Arc<dyn Homes>);
    let (again, seen) = next.build_on(dir, log).await;
    let thread = started(&again, Some(project)).await;
    let workspace = thread.workspace.clone().unwrap();
    assert_eq!(workspace.computer.kind, ComputerKind::Person);
    assert_eq!(PathBuf::from(&workspace.root), root);
    turn(&again, thread.id, "hi").await.unwrap();
    assert_eq!(later.asked(), 0, "nothing asked of their computer");
    let opening = match seen
        .lock()
        .unwrap()
        .first()
        .and_then(|sent| sent.messages.first())
    {
        Some(Message::User { parts }) => parts.first().cloned(),
        _ => None,
    };
    assert!(
        matches!(&opening, Some(Part::Text { text }) if text.contains("Be brief.")),
        "the person's own instructions, kept: {opening:?}"
    );
}

const SKILL_TEXT: &str =
    "---\nname: tidy\ndescription: Tidy the garden beds.\n---\nPrune only in winter.\n";

/// A skill on the server's own computer, and one built into glave.
fn on_the_server() -> Option<Skills> {
    let built_in = "---\nname: sheets\ndescription: Work with spreadsheets.\n---\nCount twice.\n";
    Skills::offer(vec![
        Skill {
            name: "servers-own".into(),
            description: "Only for the server's computer.".into(),
            source: SkillSource::File(PathBuf::from("/nowhere/SKILL.md")),
        },
        Skill::built_in(built_in).unwrap(),
    ])
}

#[test]
fn a_thread_on_a_person_s_computer_is_offered_the_skills_there_and_never_the_server_s() {
    let dir = folder("session-homes-skills");
    block_on!(async {
        let homes = Arc::new(Here::new(&dir.join("people")));
        let mut setup = Setup::asking(Vec::new(), Answering::NoOne);
        setup.homes = Some(Arc::clone(&homes) as Arc<dyn Homes>);
        let log = Shared::default();
        let (server, _) = setup.build_on(&dir, log.clone()).await;
        let person = server.person().unwrap();
        let own = homes.home_folder(person).join(".glave/skills/tidy");
        std::fs::create_dir_all(&own).unwrap();
        std::fs::write(own.join("SKILL.md"), SKILL_TEXT).unwrap();
        let made = Request::ProjectCreate {
            name: "Garden".into(),
            folder: Some(NewFolder::Empty),
        };
        let Reply::Project {
            project,
            folder: Some(folder),
        } = server.request(made).await.unwrap()
        else {
            panic!("a project with its folder")
        };

        // A server started later: offered as last read there, with nothing asked of it.
        let mut next = Setup::new(vec![
            answer(Some("Hello."), None),
            answer(None, Some(("skill", json!({"name": "tidy"})))),
            answer(
                None,
                Some(("write", json!({"path": "n.txt", "content": "x"}))),
            ),
            answer(Some("Done."), None),
            answer(Some("Hello again."), None),
        ]);
        next.seed = 2;
        next.settings.person = Some(person);
        next.projects = Some(Arc::clone(server.projects()));
        next.skills = on_the_server();
        let later = Arc::new(Here::new(&dir.join("people")));
        next.homes = Some(Arc::clone(&later) as Arc<dyn Homes>);
        let (again, seen) = next.build_on(&dir, log).await;
        let thread = started(&again, Some(project.id)).await;
        turn(&again, thread.id, "hi").await.unwrap();
        assert_eq!(later.asked(), 0, "nothing asked of their computer");
        let offered = json(&seen).pop().unwrap();
        assert!(offered.contains("Tidy the garden beds."), "{offered}");
        assert!(offered.contains("glave:sheets"), "glave's own: {offered}");
        assert!(!offered.contains("servers-own"), "{offered}");

        // Loading one reads it there; and a turn that changed a file reads them all again.
        let root = PathBuf::from(&folder.checkout.path);
        let beds = root.join(".glave/skills/beds");
        std::fs::create_dir_all(&beds).unwrap();
        let text = "---\nname: beds\ndescription: Lay out new beds.\n---\nDig first.\n";
        std::fs::write(beds.join("SKILL.md"), text).unwrap();
        turn(&again, thread.id, "tidy up").await.unwrap();
        let loaded = json(&seen);
        assert!(
            loaded
                .iter()
                .any(|sent| sent.contains("Prune only in winter."))
        );
        let next_thread = started(&again, Some(project.id)).await;
        let asked = later.asked();
        turn(&again, next_thread.id, "hi").await.unwrap();
        assert_eq!(later.asked(), asked, "a turn that talks asks for nothing");
        let offered = json(&seen).pop().unwrap();
        assert!(offered.contains("Lay out new beds."), "{offered}");
    });
}

#[test]
fn a_worktree_on_a_person_s_computer_is_offered_what_its_repository_was_last_read_to_hold() {
    let dir = folder("session-homes-worktree-skills");
    block_on!(async {
        let homes = Arc::new(Here::new(&dir.join("people")));
        let mut setup = Setup::new(vec![answer(Some("Hello."), None)]);
        setup.homes = Some(Arc::clone(&homes) as Arc<dyn Homes>);
        let (server, seen) = setup.build_on(&dir, Shared::default()).await;
        let person = server.person().unwrap();
        let own = homes.home_folder(person).join(".glave/skills/tidy");
        std::fs::create_dir_all(&own).unwrap();
        std::fs::write(own.join("SKILL.md"), SKILL_TEXT).unwrap();
        let made = Request::ProjectCreate {
            name: "Garden".into(),
            folder: Some(NewFolder::Empty),
        };
        let Reply::Project {
            project,
            folder: Some(folder),
        } = server.request(made).await.unwrap()
        else {
            panic!("a project with its folder")
        };
        let committed = std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(["commit", "--allow-empty", "-m", "first"])
            .current_dir(&folder.checkout.path)
            .output()
            .unwrap();
        assert!(committed.status.success(), "{committed:?}");
        let started = Request::ThreadStart {
            project: Some(project.id),
            worktree: Some(true),
            background: false,
            computer: None,
            model: None,
            permissions: None,
            effort: None,
        };
        let Reply::Thread { thread } = server.request(started).await.unwrap() else {
            panic!("a thread")
        };
        let woken = homes.woken();
        turn(&server, thread.id, "hi").await.unwrap();
        assert_eq!(homes.woken(), woken, "a turn that talks wakes nothing");
        let offered = json(&seen).pop().unwrap();
        assert!(offered.contains("Tidy the garden beds."), "{offered}");
    });
}
