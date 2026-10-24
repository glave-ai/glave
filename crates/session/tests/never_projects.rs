//! The folders that are never a project (`specs/one-server.md`, "A folder's project"): on the
//! server's computer, `project/create` and `project/folder/set` refuse its home folder, the
//! folders above it and its temporary folder, each with a sentence saying what to pick
//! instead, and `folder/list` says so of each of them, so a client never offers it, though the
//! folders inside may be one. The computer names its home through a link, as macOS names its
//! temporary folder, and the rule holds of where the link leads.

use std::path::{Path, PathBuf};

use glave_session::Answering;
use glave_session::protocol::{NewFolder, Reply, Request, code};

use crate::support::*;

const HOME_HOLDS_CHATS: &str = "your home folder holds your Chats, so it is no project's \
                                folder: pick a folder inside it";

/// A server whose computer's home folder is `<base>/home/ada`, named through the link
/// `<base>/ada`, and whose temporary folder is `<base>/tmp`; and those two folders.
async fn server_in(base: &Path) -> (Tested, PathBuf, PathBuf) {
    let (home, temporary) = (base.join("home/ada"), base.join("tmp"));
    for made in [&home.join("code/app"), &temporary] {
        std::fs::create_dir_all(made).unwrap();
    }
    std::os::unix::fs::symlink(&home, base.join("ada")).unwrap();
    let mut setup = Setup::asking(Vec::new(), Answering::NoOne);
    setup.variables = vec![
        ("HOME".into(), base.join("ada")),
        ("TMPDIR".into(), temporary.clone()),
    ];
    let (server, _) = setup.build(&home.join("code/app")).await;
    (server, home, temporary)
}

fn create_at(path: &Path) -> Request {
    Request::ProjectCreate {
        name: "app".into(),
        folder: Some(NewFolder::Path {
            path: path.to_string_lossy().into_owned(),
        }),
    }
}

fn list(path: &Path) -> Request {
    Request::FolderList {
        path: Some(path.to_string_lossy().into_owned()),
    }
}

#[test]
fn the_home_folder_those_above_it_and_the_temporary_one_are_never_a_project_s_folder() {
    let base = folder("never");
    block_on!(async {
        let (server, home, temporary) = server_in(&base).await;
        let above_home = "this folder holds your home folder, so it is no project's folder: \
                          pick a folder inside your home folder, or another";
        let temporary_folder = "this is the computer's temporary folder, which it empties, so \
                                it is no project's folder: pick another folder";
        for (folder, said) in [
            (home.clone(), HOME_HOLDS_CHATS),
            (base.join("ada"), HOME_HOLDS_CHATS),
            (home.join("../ada/"), HOME_HOLDS_CHATS),
            (base.join("home"), above_home),
            (base.to_path_buf(), above_home),
            (temporary.clone(), temporary_folder),
        ] {
            let refused = server.request(create_at(&folder)).await.unwrap_err();
            assert_eq!(refused.code, code::INVALID_PARAMS, "{folder:?}");
            assert_eq!(&*refused.message, said, "{folder:?}");
            let Reply::Folders { refused, .. } = server.request(list(&folder)).await.unwrap()
            else {
                panic!("folders");
            };
            assert_eq!(refused.as_deref(), Some(said), "{folder:?}");
        }
        let listed = server.request(Request::ProjectList { limit: None }).await;
        let Ok(Reply::Projects { projects, .. }) = listed else {
            panic!("projects");
        };
        assert!(projects.iter().all(|project| project.name != "app"));

        // A folder inside any of them may be one.
        let code_folder = home.join("code");
        let Reply::Folders { refused, .. } = server.request(list(&code_folder)).await.unwrap()
        else {
            panic!("folders");
        };
        assert_eq!(refused, None);
        let made = server.request(create_at(&code_folder)).await.unwrap();
        assert!(
            matches!(
                made,
                Reply::Project {
                    folder: Some(_),
                    ..
                }
            ),
            "{made:?}"
        );
    });
}

#[test]
fn a_project_whose_folder_moved_is_given_none_of_them_as_its_new_place() {
    let base = folder("never-moved");
    block_on!(async {
        let (server, home, _) = server_in(&base).await;
        let code_folder = home.join("code");
        let made = server.request(create_at(&code_folder)).await.unwrap();
        let Reply::Project { project, .. } = made else {
            panic!("a project");
        };
        std::fs::rename(&code_folder, home.join("moved")).unwrap();
        let moved = Request::ProjectFolderSet {
            project: project.id,
            path: home.to_string_lossy().into_owned(),
        };
        let refused = server.request(moved).await.unwrap_err();
        assert_eq!(refused.code, code::INVALID_PARAMS);
        assert_eq!(&*refused.message, HOME_HOLDS_CHATS);
    });
}
