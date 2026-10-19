//! Projects own their folders: `project/create` makes a project from a folder already here, a
//! repository cloned with the git host connected for it, or a new empty repository; a thread
//! in it works in its folder, or a worktree of it when asked, opening with what the folder's
//! instructions say; a person's Chats get scratch folders; project names stay apart per owner;
//! and only the person on this computer gives a folder or sees the folders on it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glave_event::{
    At, Auth, AuthMethod, Body, Checkout, EventId, FolderSource, Header, Opened, ProjectFolderSet,
    Seq, ThreadId, decode, encode,
};
use glave_harness::ProjectFolders;
use glave_ports::computer::{CredentialHost, Forge, GitCredential};
use glave_ports::testing::{FakeGitHost, FixedGitCredentials, fake_secret};
use glave_ports::{BoxFuture, Caller, EventLog, ProjectId};
use glave_session::Answering;
use glave_session::protocol::{Meta, NewFolder, ProjectFolder, Reply, Request, code};
use glave_session::signin::person;
use serde_json::json;

use crate::support::*;

fn canonical(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

fn create(name: &str, folder: Option<NewFolder>) -> Request {
    Request::ProjectCreate {
        name: name.into(),
        members: Vec::new(),
        folder,
    }
}

/// The project and folder a `project/*` request answered with.
fn made(reply: Reply) -> (glave_ports::Project, Option<ProjectFolder>) {
    match reply {
        Reply::Project { project, folder } => (project, folder),
        other => panic!("a project, not {other:?}"),
    }
}

/// Starts a thread in `project`, in a worktree of its own when `worktree` says.
async fn start_in(server: &Tested, project: ProjectId, worktree: Option<bool>) -> ThreadId {
    let started = Request::ThreadStart {
        project: Some(project),
        worktree,
        background: false,
        computer: None,
        model: None,
        approval: None,
    };
    match server.request(started).await.unwrap() {
        Reply::Thread { thread } => thread.id,
        other => panic!("a thread, not {other:?}"),
    }
}

#[test]
fn a_project_made_from_a_folder_here_works_in_it_and_the_folder_leads_back_to_it() {
    let dir = folder("projects-path");
    let garden = dir.with_extension("garden");
    let _ = std::fs::remove_dir_all(&garden);
    std::fs::create_dir_all(&garden).unwrap();
    block_on!(async {
        let answers = vec![
            answer(
                None,
                Some(("write", json!({"path": "plan.md", "content": "dig"}))),
            ),
            answer(Some("Planned."), None),
        ];
        let server = Setup::new(answers).build(&dir).await.0;
        let path = garden.to_string_lossy().into_owned();
        let asked = NewFolder::Path { path: path.clone() };
        let (project, folder) = made(server.request(create("Garden", Some(asked))).await.unwrap());
        let folder = folder.unwrap();
        assert_eq!(
            folder.source,
            FolderSource::Local,
            "no repository it comes from"
        );
        assert_eq!(folder.checkout.path, canonical(&garden));
        let thread = start_in(&server, project.id, None).await;
        let (place, root) = opened(&server, thread).await;
        assert!(matches!(place, Opened::Folder { .. }), "{place:?}");
        assert_eq!(root, canonical(&garden), "in the folder itself");
        turn(&server, thread, "plan").await.unwrap();
        assert_eq!(
            std::fs::read_to_string(garden.join("plan.md")).unwrap(),
            "dig"
        );
        // The same folder again is the same project, as glave run there finds it.
        let again = NewFolder::Path { path };
        let (same, _) = made(server.request(create("Other", Some(again))).await.unwrap());
        assert_eq!((same.id, same.name), (project.id, "Garden".to_owned()));
        let (_, got) = made(
            server
                .request(Request::ProjectGet {
                    project: project.id,
                })
                .await
                .unwrap(),
        );
        assert_eq!(got.unwrap().checkout.path, canonical(&garden));
        let missing = NewFolder::Path {
            path: dir.join("missing").to_string_lossy().into_owned(),
        };
        let refused = server
            .request(create("Nowhere", Some(missing)))
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::INVALID_PARAMS);
        assert!(refused.message.contains("no folder"), "{refused}");
    });
}

#[test]
fn a_thread_in_a_project_works_in_a_worktree_of_its_folder_when_asked() {
    let dir = folder("projects-worktree");
    let repo = repository(&dir.with_extension("repos"), "app");
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.worktrees = Some(dir.with_extension("worktrees"));
        let server = setup.build(&dir).await.0;
        let path = repo.to_string_lossy().into_owned();
        let (project, _) = made(
            server
                .request(create("App", Some(NewFolder::Path { path })))
                .await
                .unwrap(),
        );
        let thread = start_in(&server, project.id, Some(true)).await;
        let (place, root) = opened(&server, thread).await;
        let Opened::Worktree { repository, branch } = place else {
            panic!("a worktree, not {place:?}")
        };
        assert_eq!(repository, canonical(&repo));
        assert!(branch.starts_with("glave/"), "{branch}");
        assert!(Path::new(&root).join("AGENTS.md").exists());
        assert!(
            !root.starts_with(&canonical(&repo)),
            "outside the repository"
        );
        let in_place = start_in(&server, project.id, None).await;
        let (place, root) = opened(&server, in_place).await;
        assert!(
            matches!(place, Opened::Folder { .. }),
            "in place by default"
        );
        assert_eq!(root, canonical(&repo));
    });
}

#[test]
fn an_empty_project_is_a_new_repository_named_after_it_and_names_stay_apart() {
    let dir = folder("projects-empty");
    let projects = dir.with_extension("projects");
    let _ = std::fs::remove_dir_all(&projects);
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.project_base = Some(projects.clone());
        let server = setup.build(&dir).await.0;
        let (first, folder) = made(
            server
                .request(create("My App", Some(NewFolder::Empty)))
                .await
                .unwrap(),
        );
        let folder = folder.unwrap();
        assert_eq!(folder.source, FolderSource::Local);
        assert_eq!(folder.checkout.path, canonical(&projects.join("My-App")));
        assert!(projects.join("My-App/.git").is_dir(), "a new repository");
        let (second, folder) = made(
            server
                .request(create("my app", Some(NewFolder::Empty)))
                .await
                .unwrap(),
        );
        assert_ne!(second.id, first.id);
        assert_eq!(
            second.name, "my app 2",
            "the next free name, never an error"
        );
        // A folder of its own, `my-app` or `my-app-2` as the file system tells names apart.
        let path = PathBuf::from(folder.unwrap().checkout.path);
        assert_ne!(path, PathBuf::from(canonical(&projects.join("My-App"))));
        assert!(path.join(".git").is_dir(), "a new repository of its own");
        assert_eq!(path.parent().unwrap(), Path::new(&canonical(&projects)));
        let renamed = server
            .request(Request::ProjectRename {
                project: second.id,
                name: "MY APP".into(),
            })
            .await
            .unwrap_err();
        assert_eq!(renamed.code, code::INVALID_PARAMS);
        assert!(renamed.message.contains("choose another name"), "{renamed}");
        // A thread in a new repository with no commit yet works in its folder.
        let thread = start_in(&server, first.id, None).await;
        let (_, root) = opened(&server, thread).await;
        assert_eq!(root, canonical(&projects.join("My-App")));
    });
}

#[test]
fn a_project_from_a_repository_is_cloned_signing_in_with_its_git_host() {
    let dir = folder("projects-clone");
    let source = repository(&dir.with_extension("source"), "app");
    let bare = dir.with_extension("bare");
    let _ = std::fs::remove_dir_all(&bare);
    sh(
        &dir,
        &format!(
            "git clone -q --bare '{}' '{}' && git -C '{}' update-server-info",
            source.display(),
            bare.display(),
            bare.display()
        ),
    );
    let projects = dir.with_extension("projects");
    let _ = std::fs::remove_dir_all(&projects);
    // A token for the fake git host to sign in with; it must never show.
    let token = fake_secret("ghp_");
    let host = FakeGitHost::serving(&bare, "x-access-token", &token).unwrap();
    let url = format!("{}/repo.git", host.url);
    block_on!(async {
        // Without the host connected, the host refuses and nothing is left behind.
        let mut setup = Setup::new(Vec::new());
        setup.project_base = Some(projects.clone());
        let server = setup.build(&dir).await.0;
        let refused = server
            .request(create(
                "App",
                Some(NewFolder::Repository { url: url.clone() }),
            ))
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::HOST, "{refused}");
        assert!(!refused.message.contains(&token));
        let left = std::fs::read_dir(&projects).map_or(0, Iterator::count);
        assert_eq!(left, 0, "the failed clone's folder is removed");

        let credential = GitCredential::new("x-access-token", token.as_str()).unwrap();
        let hosts = vec![CredentialHost {
            url: host.url.clone(),
            forge: Forge::Other,
        }];
        let fixed = Arc::new(FixedGitCredentials::new(hosts, credential));
        let mut setup = Setup::new(Vec::new());
        setup.project_base = Some(projects.clone());
        setup.credentials = Some(fixed.clone());
        let server = setup.build(&folder("projects-clone-again")).await.0;
        let asked = NewFolder::Repository { url: url.clone() };
        let (project, folder) = made(server.request(create("App", Some(asked))).await.unwrap());
        let folder = folder.unwrap();
        assert_eq!(folder.source, FolderSource::Repository { url });
        let clone = PathBuf::from(&folder.checkout.path);
        assert_eq!(
            std::fs::read_to_string(clone.join("AGENTS.md")).unwrap(),
            "Use tabs.\n"
        );
        let address = host.url.trim_start_matches("http://");
        assert!(
            fixed.asked().iter().any(|ask| ask.host == address),
            "git asked the connected host's credential through the helper"
        );
        let config = std::fs::read_to_string(clone.join(".git/config")).unwrap();
        assert!(
            !config.contains(&token),
            "the token is never kept in the clone"
        );
        let thread = start_in(&server, project.id, None).await;
        assert_eq!(opened(&server, thread).await.1, folder.checkout.path);
    });
}

/// Instructions that say which folder a thread reads them for.
struct Told;

impl ProjectFolders for Told {
    fn for_folder<'a>(
        &'a self,
        root: &'a Path,
        project: Option<&'a Path>,
        _trusting: bool,
    ) -> BoxFuture<'a, Option<String>> {
        let text = match project {
            Some(folder) => format!("Rules of {} at {}", folder.display(), root.display()),
            None => "Your own rules".to_owned(),
        };
        Box::pin(std::future::ready(Some(text)))
    }
}

#[test]
fn a_thread_opens_with_its_own_folder_s_instructions_and_a_chat_with_the_person_s() {
    let dir = folder("projects-instructions");
    let other = dir.with_extension("other");
    std::fs::create_dir_all(&other).unwrap();
    block_on!(async {
        let answers = vec![answer(Some("Yes."), None), answer(Some("Hi."), None)];
        let mut setup = Setup::new(answers);
        setup.project_folders = Some(Arc::new(Told));
        let (server, seen) = setup.build(&dir).await;
        let path = other.to_string_lossy().into_owned();
        let (project, _) = made(
            server
                .request(create("Other", Some(NewFolder::Path { path })))
                .await
                .unwrap(),
        );
        let thread = start_in(&server, project.id, None).await;
        turn(&server, thread, "go").await.unwrap();
        let grace = Caller {
            person: Some(person("https://id.example.com", "grace").unwrap()),
            auth: None,
            trace: None,
        };
        let chat = server
            .request_as(
                &grace,
                Request::ThreadStart {
                    project: None,
                    worktree: None,
                    background: false,
                    computer: None,
                    model: None,
                    approval: None,
                },
            )
            .await
            .unwrap();
        let Reply::Thread { thread: chat } = chat else {
            panic!("a thread")
        };
        let read = Request::ThreadEvents {
            thread: chat.id,
            after: None,
            limit: None,
        };
        let Reply::Events { events, .. } = server.request_as(&grace, read).await.unwrap() else {
            panic!("events")
        };
        let bound = decode(events[1].clone()).unwrap().body;
        assert!(
            matches!(&bound, Body::WorkspaceBound(bound) if matches!(bound.opened, Opened::Scratch { .. })),
            "{bound:?}"
        );
        server
            .request_as(
                &grace,
                Request::TurnStart {
                    thread: chat.id,
                    prompt: "hello".into(),
                    background: false,
                    budget: None,
                    model: None,
                    approval: None,
                    meta: Meta::default(),
                },
            )
            .await
            .unwrap();
        let sent = json(&seen);
        let folder = canonical(&other);
        assert!(
            sent[0].contains(&format!("Rules of {folder} at {folder}")),
            "{}",
            sent[0]
        );
        assert!(sent[1].contains("Your own rules"), "{}", sent[1]);
        assert!(!sent[1].contains("Rules of"));
    });
}

#[test]
fn the_person_here_sees_the_folders_in_a_folder_by_name() {
    let dir = folder("projects-here");
    for name in ["beta", "Alpha", ".hidden"] {
        std::fs::create_dir_all(dir.join(name)).unwrap();
    }
    std::fs::write(dir.join("notes.txt"), "a file").unwrap();
    block_on!(async {
        let server = server(&dir, Vec::new(), Answering::NoOne).await;
        let listed = server
            .request(Request::FolderList {
                path: Some(dir.to_string_lossy().into_owned()),
            })
            .await
            .unwrap();
        let Reply::Folders {
            path,
            parent,
            folders,
        } = listed
        else {
            panic!("folders, not {listed:?}")
        };
        assert_eq!(path, canonical(&dir));
        assert_eq!(
            folders,
            ["Alpha", "beta"],
            "folders only, hidden ones left out"
        );
        let up = Path::new(&path)
            .parent()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(parent, Some(up));
        let relative = server
            .request(Request::FolderList {
                path: Some("code".into()),
            })
            .await
            .unwrap_err();
        assert_eq!(relative.code, code::INVALID_PARAMS);
    });
}

#[test]
fn a_caller_signed_in_elsewhere_gives_no_folder_and_sees_none() {
    let dir = folder("projects-elsewhere-caller");
    block_on!(async {
        let server = server(&dir, Vec::new(), Answering::NoOne).await;
        let elsewhere = Caller {
            person: Some(person("https://id.example.com", "grace").unwrap()),
            auth: Some(Auth {
                method: AuthMethod::Oidc,
                issuer: Some("https://id.example.com".into()),
                subject: Some("grace".into()),
                session: None,
            }),
            trace: None,
        };
        let path = dir.to_string_lossy().into_owned();
        for request in [
            Request::FolderList { path: None },
            create("Theirs", Some(NewFolder::Path { path })),
            create("Theirs", Some(NewFolder::Empty)),
        ] {
            let refused = server.request_as(&elsewhere, request).await.unwrap_err();
            assert_eq!(refused.code, code::NOT_ALLOWED, "{refused}");
        }
        // Without a folder, they may make a project; its threads get scratch folders.
        let (project, folder) = made(
            server
                .request_as(&elsewhere, create("Theirs", None))
                .await
                .unwrap(),
        );
        assert!(folder.is_none());
        let started = server
            .request_as(
                &elsewhere,
                Request::ThreadStart {
                    project: Some(project.id),
                    worktree: Some(true),
                    background: false,
                    computer: None,
                    model: None,
                    approval: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(
            started.code,
            code::WORKSPACE,
            "no folder to make a worktree of"
        );
    });
}

#[test]
fn the_folder_glave_runs_in_becomes_its_project_s_folder_from_its_repository() {
    let dir = folder("projects-keep");
    let repo = repository(&dir.with_extension("repos"), "app");
    sh(
        &repo,
        "git remote add origin https://example.com/acme/app.git",
    );
    block_on!(async {
        let server = Setup::new(Vec::new()).build(&dir).await.0;
        server
            .keep_folder(scope().project(), &repo.to_string_lossy())
            .await
            .unwrap();
        let (_, folder) = made(
            server
                .request(Request::ProjectGet {
                    project: scope().project(),
                })
                .await
                .unwrap(),
        );
        let folder = folder.unwrap();
        let url = "https://example.com/acme/app.git".to_owned();
        assert_eq!(folder.source, FolderSource::Repository { url });
        assert_eq!(folder.checkout.path, canonical(&repo));
        let thread = start(&server, false).await;
        assert_eq!(opened(&server, thread).await.1, canonical(&repo));
    });
}

#[test]
fn a_project_whose_folder_is_on_another_computer_starts_no_thread_here() {
    let dir = folder("projects-elsewhere");
    block_on!(async {
        let log = Shared::default();
        let server = Setup::new(Vec::new()).build_on(&dir, log.clone()).await.0;
        let (project, _) = made(server.request(create("Far", None)).await.unwrap());
        let at = At::try_from("2026-10-02T00:00:00.000Z".to_owned()).unwrap();
        let header = Header {
            id: EventId::new(at.timestamp(), [8; 10]).unwrap(),
            thread: glave_session::project_ledger().unwrap(),
            seq: Seq::FIRST,
            at,
        };
        let set = ProjectFolderSet {
            folder: uuid::Uuid::from_u128(4),
            source: FolderSource::Local,
            checkout: Checkout {
                computer: uuid::Uuid::from_u128(99),
                path: "/Users/ada/far".into(),
            },
            by: None,
        };
        let scope = glave_ports::Scope::glaves_own(scope().org(), project.id);
        let raw = encode(header, &set).unwrap();
        log.append(&scope, &[raw]).await.unwrap();
        let refused = server
            .request(Request::ThreadStart {
                project: Some(project.id),
                worktree: None,
                background: false,
                computer: None,
                model: None,
                approval: None,
            })
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::WORKSPACE);
        assert!(refused.message.contains("another computer"), "{refused}");
    });
}
