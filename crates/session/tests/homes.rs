//! Each person's own computer, where a deployment gives people one (`specs/computers.md`, "A
//! person's computer in the cloud"): their threads work there, in a scratch folder on it,
//! recorded as on their own computer; opening the app, starting a chat or a thread in a project
//! there, reading it and a turn that only talks never wake it, and the first tool call that
//! needs it does, once, while the turn holds it. A computer of this machine stands in for the
//! cluster's.

mod here;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glave_event::{
    Body, ComputerKind, Part, PrincipalId, RemovedReason, ThreadId, WorkspaceRemoved, decode,
};
use glave_harness::{Homes, Skill, SkillSource, Skills};
use glave_ports::computer::{DiskUse, Sandbox, SandboxLevel};
use glave_ports::{Message, ProjectId};
use glave_session::Answering;

use glave_session::protocol::{NewFolder, ProjectFolder, Reply, Request, Show, ThreadSummary};
use serde_json::json;

use crate::support::*;

pub(crate) use here::Here;

/// A server on `log` whose person has a computer of their own, with no answers to give.
pub(crate) async fn served_with_homes(
    dir: &Path,
    log: &Shared,
) -> (Arc<Here>, Tested<Shared>, PrincipalId) {
    let homes = Arc::new(Here::new(&dir.join("people")));
    let mut setup = Setup::asking(Vec::new(), Answering::NoOne);
    setup.homes = Some(Arc::clone(&homes) as Arc<dyn Homes>);
    let (server, _) = setup.build_on(dir, log.clone()).await;
    let person = server.person().unwrap();
    (homes, server, person)
}

/// What the server tells every client of commands' walls goes by people's computers, not by its
/// own: a sandbox there says nothing, however this one stands, and none says what that means.
#[test]
fn the_server_says_how_commands_stand_on_a_person_s_computer_not_on_its_own() {
    let dir = folder("session-homes-walls");
    block_on!(async {
        for (sandbox, says) in [
            (
                Some(Sandbox {
                    level: SandboxLevel::Project,
                    network: true,
                    home: None,
                    glave_home: None,
                }),
                json!(null),
            ),
            (
                None,
                json!(
                    "Commands here run without glave's sandbox: in Auto they can read and change \
                     anything in your home, and Ask first asks before every change and command."
                ),
            ),
        ] {
            let mut homes = Here::new(&dir.join("people"));
            homes.sandbox = sandbox;
            let mut setup = Setup::asking(Vec::new(), Answering::NoOne);
            setup.homes = Some(Arc::new(homes) as Arc<dyn Homes>);
            let (server, _) = setup.build_on(&dir, Shared::default()).await;
            let told = server.request(Request::Discover {}).await.unwrap();
            let told = serde_json::to_value(&told).unwrap();
            assert_eq!(told["unsandboxed"], says);
        }
    });
}

/// A thread `server` started for its person, in `project` or a chat of their own.
pub(crate) async fn started(server: &Tested<Shared>, project: Option<ProjectId>) -> ThreadSummary {
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
        let log = Shared::default();
        let (homes, server, person) = served_with_homes(&dir, &log).await;
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
            ..
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

/// `next` as a server started later on `server`'s store, for the same person, and the computers
/// it reaches.
pub(crate) fn later_on(mut next: Setup, dir: &Path, server: &Tested<Shared>) -> (Arc<Here>, Setup) {
    next.seed = 2;
    next.settings.person = server.person();
    next.projects = Some(Arc::clone(server.projects()));
    let later = Arc::new(Here::new(&dir.join("people")));
    next.homes = Some(Arc::clone(&later) as Arc<dyn Homes>);
    (later, next)
}

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

/// A new project on `person`'s computer, read there with a skill of their own already in place.
async fn garden_with_a_skill(
    homes: &Here,
    server: &Tested<Shared>,
    person: PrincipalId,
) -> (glave_ports::Project, ProjectFolder) {
    let own = homes.home_folder(person).join(".glave/skills/tidy");
    std::fs::create_dir_all(&own).unwrap();
    std::fs::write(own.join("SKILL.md"), SKILL_TEXT).unwrap();
    new_garden(server).await
}

/// A new project with a folder of its own on `server`'s person's computer.
pub(crate) async fn new_garden(server: &Tested<Shared>) -> (glave_ports::Project, ProjectFolder) {
    let made = Request::ProjectCreate {
        name: "Garden".into(),
        folder: Some(NewFolder::Empty),
    };
    let reply = server.request(made).await.unwrap();
    let Reply::Project {
        project,
        folder: Some(folder),
        ..
    } = reply
    else {
        panic!("a project with its folder, not {reply:?}")
    };
    (project, folder)
}

#[test]
fn a_thread_on_a_person_s_computer_is_offered_the_skills_there_and_never_the_server_s() {
    let dir = folder("session-homes-skills");
    block_on!(async {
        let log = Shared::default();
        let (homes, server, person) = served_with_homes(&dir, &log).await;
        let (project, folder) = garden_with_a_skill(&homes, &server, person).await;

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
        next.skills = on_the_server();
        let (later, next) = later_on(next, &dir, &server);
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
        let setup = Setup::new(vec![answer(Some("Hello."), None)]);
        let homes = Arc::new(Here::new(&dir.join("people")));
        let setup = Setup {
            homes: Some(Arc::clone(&homes) as Arc<dyn Homes>),
            ..setup
        };
        let (server, seen) = setup.build_on(&dir, Shared::default()).await;
        let person = server.person().unwrap();
        let (project, folder) = garden_with_a_skill(&homes, &server, person).await;
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

/// A worktree thread of `project`, as `started` in a worktree of its own: its id and its root.
async fn in_a_worktree(server: &Tested<Shared>, project: ProjectId) -> (ThreadId, PathBuf) {
    let start = Request::ThreadStart {
        project: Some(project),
        worktree: Some(true),
        background: false,
        computer: None,
        model: None,
        permissions: None,
        effort: None,
    };
    let reply = server.request(start).await.unwrap();
    let Reply::Thread { thread } = reply else {
        panic!("a thread, not {reply:?}")
    };
    let root = thread.workspace.unwrap().root;
    (thread.id, PathBuf::from(root))
}

/// What `thread` recorded of its worktree or scratch folder being removed.
async fn removals(server: &Tested<Shared>, thread: ThreadId) -> Vec<WorkspaceRemoved> {
    events(server, thread)
        .await
        .into_iter()
        .filter_map(|raw| match decode(raw).unwrap().body {
            Body::WorkspaceRemoved(removed) => Some(removed),
            _ => None,
        })
        .collect()
}

/// What `thread` recorded of its worktree being removed, by why.
async fn removed_because(server: &Tested<Shared>, thread: ThreadId) -> Vec<RemovedReason> {
    let removals = removals(server, thread).await;
    removals.into_iter().map(|removed| removed.reason).collect()
}

/// A chat of `server`'s person that wrote a file in its scratch folder: its id and its root.
async fn a_chat_that_wrote(server: &Tested<Shared>) -> (ThreadId, PathBuf) {
    let chat = started(server, None).await;
    turn(server, chat.id, "write it down").await.unwrap();
    let root = PathBuf::from(chat.workspace.unwrap().root);
    assert!(root.join("n.txt").exists(), "{}", root.display());
    (chat.id, root)
}

/// A full computer moves the scratch folders of archived chats to the Trash, where the computer
/// empties them as it needs room, and never an active chat's, however long it has been idle or
/// however lately brought back: what a scratch folder holds is nowhere else. A chat brought back
/// after finds its files again.
#[test]
fn a_full_computer_moves_archived_chats_scratch_folders_to_the_trash() {
    let dir = folder("session-homes-scratch");
    block_on!(async {
        let homes = Arc::new(Here::new(&dir.join("people")));
        let wrote = ("write", json!({"path": "n.txt", "content": "x"}));
        let mut answers = Vec::new();
        for said in [
            "Written.",
            "Written too.",
            "Written three.",
            "Written four.",
            "Again.",
            "Still.",
        ] {
            answers.push(answer(None, Some(wrote.clone())));
            answers.push(answer(Some(said), None));
        }
        let other = ("write", json!({"path": "m.txt", "content": "y"}));
        answers.push(answer(None, Some(other)));
        answers.push(answer(Some("Back."), None));
        let mut setup = Setup::new(answers);
        setup.homes = Some(Arc::clone(&homes) as Arc<dyn Homes>);
        let clock = setup.clock.clone();
        let (server, _) = setup.build_on(&dir, Shared::default()).await;
        let (archived, archived_root) = a_chat_that_wrote(&server).await;
        let (back, back_root) = a_chat_that_wrote(&server).await;
        let (idle, idle_root) = a_chat_that_wrote(&server).await;
        let (writing, _) = a_chat_that_wrote(&server).await;
        for archive in [archived, back] {
            let archive = Request::ThreadArchive { thread: archive };
            server.request(archive).await.unwrap();
        }
        let unarchive = Request::ThreadUnarchive { thread: back };
        server.request(unarchive).await.unwrap();
        // Eight days on, the server holds none of them open.
        clock.advance_seconds(8 * 24 * 60 * 60);
        server.tick().await.unwrap();

        // Full until the archived chat's folder goes: it goes whole, to the Trash.
        *homes.full_while.lock().unwrap() = Some(archived_root.clone());
        turn(&server, writing, "write it down").await.unwrap();
        let removed = removals(&server, archived).await;
        assert_eq!(removed.len(), 1, "{removed:?}");
        assert_eq!(removed[0].reason, RemovedReason::Archived);
        let trashed = PathBuf::from(removed[0].trash.as_deref().expect("in the Trash"));
        assert_eq!(std::fs::read_to_string(trashed.join("n.txt")).unwrap(), "x");
        assert!(!archived_root.exists());

        // Full for good: an idle chat's folder stays, as does the one at work.
        let marker = dir.join("full");
        std::fs::write(&marker, "").unwrap();
        *homes.full_while.lock().unwrap() = Some(marker);
        turn(&server, writing, "and again").await.unwrap();
        assert_eq!(removals(&server, idle).await, []);
        assert!(idle_root.join("n.txt").exists());
        assert_eq!(removals(&server, back).await, []);
        assert!(
            back_root.join("n.txt").exists(),
            "brought back before it went"
        );

        // Brought back, the archived chat finds its files where it left them.
        let unarchive = Request::ThreadUnarchive { thread: archived };
        server.request(unarchive).await.unwrap();
        turn(&server, archived, "write another").await.unwrap();
        assert!(archived_root.join("m.txt").exists());
        assert_eq!(
            std::fs::read_to_string(archived_root.join("n.txt")).unwrap(),
            "x"
        );
        assert!(!trashed.exists(), "out of the Trash");
    });
}

#[test]
fn a_full_computer_frees_the_worktrees_of_old_threads_who_open_them_again_when_they_come_back() {
    let dir = folder("session-homes-full");
    block_on!(async {
        let homes = Arc::new(Here::new(&dir.join("people")));
        let wrote = ("write", json!({"path": "n.txt", "content": "x"}));
        let mut setup = Setup::new(vec![
            answer(Some("Busy, then quiet."), None),
            answer(None, Some(wrote.clone())),
            answer(Some("Written."), None),
            answer(None, Some(wrote)),
            answer(Some("Written again."), None),
            answer(Some("Back."), None),
        ]);
        setup.homes = Some(Arc::clone(&homes) as Arc<dyn Homes>);
        let clock = setup.clock.clone();
        let (server, _) = setup.build_on(&dir, Shared::default()).await;
        let person = server.person().unwrap();
        let (project, folder) = garden_with_a_skill(&homes, &server, person).await;
        // Everything on a remote, so what's in a worktree can be fetched again.
        let root = PathBuf::from(&folder.checkout.path);
        let bare = dir.join("origin.git");
        for args in [
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "commit",
                "--allow-empty",
                "-qm",
                "first",
            ][..],
            &["clone", "-q", "--bare", ".", bare.to_str().unwrap()][..],
            &["remote", "add", "origin", bare.to_str().unwrap()][..],
            &["fetch", "-q", "origin"][..],
        ] {
            let done = std::process::Command::new("git")
                .args(args)
                .current_dir(&root)
                .output()
                .unwrap();
            assert!(done.status.success(), "{done:?}");
        }
        let (older, older_root) = in_a_worktree(&server, project.id).await;
        let (archived, archived_root) = in_a_worktree(&server, project.id).await;
        let (recent, recent_root) = in_a_worktree(&server, project.id).await;
        server
            .request(Request::ThreadArchive { thread: archived })
            .await
            .unwrap();

        // Eight days on, what's quiet is let go of, and one thread has just worked.
        clock.advance_seconds(8 * 24 * 60 * 60);
        turn(&server, recent, "hi").await.unwrap();
        server.tick().await.unwrap();
        let writing = started(&server, Some(project.id)).await;
        let says_full = |path: &Path| {
            *homes.full_while.lock().unwrap() = Some(path.to_path_buf());
        };

        // Full until the archived thread's goes: it goes first, and that's enough.
        says_full(&archived_root);
        turn(&server, writing.id, "write it down").await.unwrap();
        assert_eq!(
            removed_because(&server, archived).await,
            [RemovedReason::Archived]
        );
        assert!(!archived_root.exists());
        assert_eq!(removed_because(&server, older).await, []);
        assert!(older_root.exists(), "it stopped once there was room");

        // Full for good: the idle one goes, and never one that worked within the week.
        says_full(&root);
        turn(&server, writing.id, "and again").await.unwrap();
        assert_eq!(removed_because(&server, older).await, [RemovedReason::Idle]);
        assert!(!older_root.exists());
        assert_eq!(removed_because(&server, recent).await, []);
        assert!(recent_root.exists(), "it worked within the week");

        // Back, it opens its worktree again.
        *homes.full_while.lock().unwrap() = None;
        turn(&server, older, "hello again").await.unwrap();
        assert!(older_root.exists(), "{}", older_root.display());
    });
}

/// Past 90%, with nothing more it can free, the server tells the person what uses their disk, as a
/// fact on their thread, largest first, and asks nothing more of the computer to say it.
#[test]
fn a_person_is_told_what_uses_their_disk_once_nothing_more_can_go() {
    let dir = folder("session-homes-disk");
    block_on!(async {
        let wrote = ("write", json!({"path": "n.txt", "content": "x"}));
        let answers = vec![
            answer(None, Some(wrote.clone())),
            answer(Some("Done."), None),
            answer(None, Some(wrote)),
            answer(Some("Done."), None),
        ];
        let homes = Arc::new(Here::new(&dir.join("people")));
        let mut setup = Setup::new(answers);
        setup.homes = Some(Arc::clone(&homes) as Arc<dyn Homes>);
        let (server, _) = setup.build_on(&dir, Shared::default()).await;
        let (project, _) = new_garden(&server).await;
        homes.largest.set(vec![DiskUse {
            path: "films/long".to_owned(),
            bytes: 3_000_000,
        }]);
        let marker = dir.join("full");
        std::fs::write(&marker, "").unwrap();
        *homes.full_while.lock().unwrap() = Some(marker.clone());

        let thread = started(&server, Some(project.id)).await;
        assert_eq!(thread.disk, None, "nothing looked at yet");
        turn(&server, thread.id, "write it down").await.unwrap();

        let asked = homes.asked();
        let read = server
            .request(Request::ThreadGet { thread: thread.id })
            .await
            .unwrap();
        let Reply::Thread { thread: read } = read else {
            panic!("a thread, not {read:?}");
        };
        assert_eq!(homes.asked(), asked, "saying so reaches nothing");
        let disk = read.disk.expect("it says so");
        assert_eq!(
            disk.told.to_string(),
            "Your computer's disk is 91% full, and glave has freed all it can."
        );
        assert_eq!((disk.size, disk.free), (100, 9));
        assert_eq!(disk.largest[0].path, "films/long");

        // Room made since: the next turn that used the computer hears it, and it says no more.
        std::fs::remove_file(&marker).unwrap();
        turn(&server, thread.id, "write it again").await.unwrap();
        let read = server
            .request(Request::ThreadGet { thread: thread.id })
            .await
            .unwrap();
        let Reply::Thread { thread: read } = read else {
            panic!("a thread, not {read:?}");
        };
        assert_eq!(read.disk, None);
    });
}
