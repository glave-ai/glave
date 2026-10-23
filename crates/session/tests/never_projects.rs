//! The folders that are never a project (`specs/one-server.md`, "A folder's project"): on the
//! server's computer, `project/create` refuses its home folder, the folders above it and its
//! temporary folder, each with a sentence saying what to pick instead, and `folder/list` says
//! so of each of them, so a client never offers it, though the folders inside may be one.

use std::path::Path;

use glave_session::Answering;
use glave_session::protocol::{NewFolder, Reply, Request, code};

use crate::support::*;

fn create_at(path: &Path) -> Request {
    Request::ProjectCreate {
        name: "app".into(),
        members: Vec::new(),
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
    let (home, temporary) = (base.join("home/ada"), base.join("tmp"));
    let code_folder = home.join("code");
    for made in [&code_folder.join("app"), &temporary] {
        std::fs::create_dir_all(made).unwrap();
    }
    block_on!(async {
        let mut setup = Setup::asking(Vec::new(), Answering::NoOne);
        setup.variables = vec![
            ("HOME".into(), home.clone()),
            ("TMPDIR".into(), temporary.clone()),
        ];
        let (server, _) = setup.build(&code_folder.join("app")).await;
        let home_holds_chats = "your home folder holds your Chats, so it is no project's folder: \
                                pick a folder inside it";
        let above_home = "this folder holds your home folder, so it is no project's folder: \
                          pick a folder inside your home folder";
        let temporary_folder = "this is the computer's temporary folder, which it empties, so \
                                it is no project's folder: pick another folder";
        for (folder, said) in [
            (home.clone(), home_holds_chats),
            (home.join("../ada/"), home_holds_chats),
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
        let listed = server
            .request(Request::ProjectList { limit: None })
            .await
            .unwrap();
        let Reply::Projects { projects, .. } = listed else {
            panic!("projects");
        };
        assert!(
            projects.iter().all(|project| project.name != "app"),
            "nothing was made"
        );

        // A folder inside any of them may be one.
        let Reply::Folders { refused, .. } = server.request(list(&code_folder)).await.unwrap()
        else {
            panic!("folders");
        };
        assert_eq!(refused, None);
        let made = server.request(create_at(&code_folder)).await.unwrap();
        let Reply::Project { project, .. } = &made else {
            panic!("a project");
        };
        // Its folder gone, it is given none of them as its new place either.
        let project = project.id;
        std::fs::rename(&code_folder, home.join("moved")).unwrap();
        let moved = Request::ProjectFolderSet {
            project,
            path: home.to_string_lossy().into_owned(),
        };
        let refused = server.request(moved).await.unwrap_err();
        assert_eq!(&*refused.message, home_holds_chats);
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
