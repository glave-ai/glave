//! A project whose folder isn't there anymore: the server says so as a fact, in plain words,
//! when the project is listed and before anything would run there, and starts nothing and
//! makes nothing in its place; the person on this computer finds it (`space/folder/set`),
//! after which its threads work in the folder's new place, or removes the project
//! (`space/remove`), its threads archived and it listed no more.

use std::path::Path;
use std::sync::Arc;

use glave_event::{Auth, AuthMethod, Body, Opened, ThreadId, decode};
use glave_ports::{Caller, EventLog, SpaceId};
use glave_session::protocol::{GoneFolder, Reply, Request, Show, code};
use glave_session::signin::person;
use serde_json::json;

use crate::support::*;

const GONE: &str = "This project's folder isn't there anymore.";

fn canonical(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

/// A fresh folder beside the test's own, called after it with `name`.
fn beside(dir: &Path, name: &str) -> std::path::PathBuf {
    let made = dir.with_extension(name);
    let _ = std::fs::remove_dir_all(&made);
    std::fs::create_dir_all(&made).unwrap();
    made
}

/// Someone signed in from elsewhere, not the person on this computer.
fn elsewhere() -> Caller {
    Caller {
        person: Some(person("https://id.example.com", "grace").unwrap()),
        auth: Some(Auth {
            method: AuthMethod::Oidc,
            issuer: Some("https://id.example.com".into()),
            subject: Some("grace".into()),
            session: None,
        }),
        trace: None,
    }
}

fn start_request(space: SpaceId, worktree: Option<bool>) -> Request {
    Request::ThreadStart {
        space: Some(space),
        worktree,
        background: false,
        computer: None,
        model: None,
        approval: None,
    }
}

async fn start_in<L: EventLog + 'static>(
    server: &Tested<L>,
    space: SpaceId,
    worktree: Option<bool>,
) -> ThreadId {
    match server
        .request(start_request(space, worktree))
        .await
        .unwrap()
    {
        Reply::Thread { thread } => thread.id,
        other => panic!("a thread, not {other:?}"),
    }
}

/// The spaces `space/list` shows, by name, and the projects it says have lost their folder.
async fn listed<L: EventLog + 'static>(server: &Tested<L>) -> (Vec<String>, Vec<GoneFolder>) {
    match server
        .request(Request::SpaceList { limit: None })
        .await
        .unwrap()
    {
        Reply::Spaces { spaces, gone } => (spaces.into_iter().map(|s| s.name).collect(), gone),
        other => panic!("spaces, not {other:?}"),
    }
}

async fn threads<L: EventLog + 'static>(
    server: &Tested<L>,
    space: SpaceId,
    show: Show,
) -> Vec<ThreadId> {
    let listed = Request::ThreadList {
        space: Some(space),
        limit: None,
        show: Some(show),
        cursor: None,
    };
    match server.request(listed).await.unwrap() {
        Reply::Threads { threads, .. } => {
            let mut ids: Vec<ThreadId> = threads.into_iter().map(|t| t.id).collect();
            ids.sort();
            ids
        }
        other => panic!("threads, not {other:?}"),
    }
}

async fn running<L: EventLog + 'static>(server: &Tested<L>, thread: ThreadId) -> usize {
    match server
        .request(Request::CommandList { thread })
        .await
        .unwrap()
    {
        Reply::Commands { commands } => commands.len(),
        other => panic!("commands, not {other:?}"),
    }
}

/// Where `thread` was last bound to work, and its root there.
async fn last_bound<L: EventLog + 'static>(
    server: &Tested<L>,
    thread: ThreadId,
) -> (Opened, String) {
    events(server, thread)
        .await
        .into_iter()
        .rev()
        .find_map(|raw| match decode(raw).unwrap().body {
            Body::WorkspaceBound(bound) => Some((bound.opened, bound.root)),
            _ => None,
        })
        .unwrap()
}

fn find(space: SpaceId, path: &Path) -> Request {
    Request::SpaceFolderSet {
        space,
        path: path.to_string_lossy().into_owned(),
    }
}

#[test]
fn a_project_whose_folder_is_gone_says_so_plainly_and_nothing_runs_or_is_made_there() {
    let dir = folder("gone-plain");
    let garden = beside(&dir, "garden");
    let log = Shared::default();
    block_on!(async {
        let (server, _) = Setup::new(vec![answer(Some("Planted."), None)])
            .build_on(&dir, log.clone())
            .await;
        let space = project(&server, "Garden", &garden).await;
        let thread = start_in(&server, space, None).await;
        turn(&server, thread, "plant").await.unwrap();
        std::fs::remove_dir_all(&garden).unwrap();

        let (names, gone) = listed(&server).await;
        assert!(names.contains(&"Garden".to_owned()), "{names:?}");
        let said = GoneFolder {
            space,
            message: GONE.into(),
        };
        assert_eq!(gone, [said]);
        // A turn in the thread held since, and a new thread: refused in those words.
        let refused = turn(&server, thread, "again").await.unwrap_err();
        assert_eq!(
            (refused.code, refused.message.as_str()),
            (code::NO_FOLDER, GONE)
        );
        let refused = server
            .request(start_request(space, None))
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::NO_FOLDER);
        assert_eq!(running(&server, thread).await, 0, "nothing runs there");

        // A server that never held the thread reads it, lists nothing running, runs nothing.
        let mut setup = Setup::new(Vec::new());
        setup.seed = 2;
        setup.settings.person = server.person();
        setup.spaces = Some(Arc::clone(server.spaces()));
        let (fresh, _) = setup.build_on(&dir, log).await;
        let got = fresh.request(Request::ThreadGet { thread }).await;
        assert!(matches!(got, Ok(Reply::Thread { .. })), "{got:?}");
        assert_eq!(running(&fresh, thread).await, 0);
        let refused = turn(&fresh, thread, "again").await.unwrap_err();
        assert_eq!(refused.code, code::NO_FOLDER);
        assert!(!garden.exists(), "nothing was made in the folder's place");
        // Archiving it still works, straight in the log.
        let archived = fresh.request(Request::ThreadArchive { thread }).await;
        assert!(matches!(archived, Ok(Reply::Done {})), "{archived:?}");
        assert_eq!(threads(&fresh, space, Show::Archived).await, [thread]);
    });
}

#[test]
fn finding_the_folder_again_moves_its_threads_there_and_the_next_turn_runs() {
    let dir = folder("gone-found");
    let garden = beside(&dir, "garden");
    let other = beside(&dir, "other");
    let moved = dir.with_extension("moved");
    let _ = std::fs::remove_dir_all(&moved);
    block_on!(async {
        let answers = vec![
            answer(Some("Planted."), None),
            answer(
                None,
                Some(("write", json!({"path": "plan.md", "content": "dig"}))),
            ),
            answer(Some("Planned."), None),
        ];
        let (server, _) = Setup::new(answers).build(&dir).await;
        let space = project(&server, "Garden", &garden).await;
        project(&server, "Other", &other).await;
        let thread = start_in(&server, space, None).await;
        turn(&server, thread, "plant").await.unwrap();

        // While the folder is where it was, there is nothing to find.
        let refused = server.request(find(space, &other)).await.unwrap_err();
        assert_eq!(refused.code, code::INVALID_PARAMS);
        std::fs::rename(&garden, &moved).unwrap();

        // Only the person on this computer gives a folder, and never another project's.
        let refused = server
            .request_as(&elsewhere(), find(space, &moved))
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED);
        let refused = server.request(find(space, &other)).await.unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED, "{refused}");

        let Reply::Space { folder, .. } = server.request(find(space, &moved)).await.unwrap() else {
            panic!("a space")
        };
        assert_eq!(folder.unwrap().checkout.path, canonical(&moved));
        assert_eq!(listed(&server).await.1, [], "nothing is gone now");
        turn(&server, thread, "plan").await.unwrap();
        assert_eq!(
            std::fs::read_to_string(moved.join("plan.md")).unwrap(),
            "dig"
        );
        let (place, root) = last_bound(&server, thread).await;
        assert!(matches!(place, Opened::Folder { .. }), "{place:?}");
        assert_eq!(
            root,
            canonical(&moved),
            "the thread works where the folder is now"
        );
        // The folder's new place leads to the project, as glave run there finds it.
        assert_eq!(project(&server, "Again", &moved).await, space);
    });
}

#[test]
fn a_thread_on_a_copy_of_a_project_that_moved_says_to_start_a_new_one() {
    let dir = folder("gone-worktree");
    let repos = beside(&dir, "repos");
    let repo = repository(&repos, "app");
    let moved = repos.join("moved");
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.worktrees = Some(dir.with_extension("worktrees"));
        let server = setup.build(&dir).await.0;
        let space = project(&server, "App", &repo).await;
        let thread = start_in(&server, space, Some(true)).await;
        std::fs::rename(&repo, &moved).unwrap();
        let refused = turn(&server, thread, "go").await.unwrap_err();
        assert_eq!(refused.code, code::NO_FOLDER);

        server.request(find(space, &moved)).await.unwrap();
        let refused = turn(&server, thread, "go").await.unwrap_err();
        assert_eq!(refused.code, code::WORKSPACE);
        assert!(
            refused.message.contains("Start a new thread to go on"),
            "{refused}"
        );
        assert!(!refused.message.contains('/'), "no path: {refused}");
    });
}

#[test]
fn removing_a_project_archives_its_threads_and_lists_it_no_more() {
    let dir = folder("gone-removed");
    let garden = beside(&dir, "garden");
    block_on!(async {
        let (server, _) = Setup::new(vec![answer(Some("Planted."), None)])
            .build(&dir)
            .await;
        let space = project(&server, "Garden", &garden).await;
        let first = start_in(&server, space, None).await;
        turn(&server, first, "plant").await.unwrap();
        let second = start_in(&server, space, None).await;
        std::fs::remove_dir_all(&garden).unwrap();

        let refused = server
            .request_as(&elsewhere(), Request::SpaceRemove { space })
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED);
        let removed = server.request(Request::SpaceRemove { space }).await;
        assert!(matches!(removed, Ok(Reply::Done {})), "{removed:?}");
        assert_eq!(threads(&server, space, Show::Active).await, []);
        let mut both = vec![first, second];
        both.sort();
        assert_eq!(threads(&server, space, Show::Archived).await, both);
        let (names, gone) = listed(&server).await;
        assert!(!names.contains(&"Garden".to_owned()), "{names:?}");
        assert_eq!(gone, []);
        // Their history is kept.
        assert!(
            kinds(&server, first)
                .await
                .contains(&"message.added".to_owned())
        );

        // Giving the project a folder again brings it back.
        std::fs::create_dir_all(&garden).unwrap();
        assert_eq!(project(&server, "Garden", &garden).await, space);
        assert!(listed(&server).await.0.contains(&"Garden".to_owned()));
    });
}
