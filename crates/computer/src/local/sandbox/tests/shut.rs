//! Someone else's commands on the person's computer read nothing of the person's home folder or
//! projects' folder but their own workspace, the toolchains there and git's settings (`specs/
//! sandbox.md`, "Whose computer"); the person's own commands read it all.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use glave_ports::block_on;
use glave_ports::computer::{
    Computers as _, Environment, RunsFor, Sandbox, SandboxLevel, SandboxNetwork, Sources,
};

use super::{committed_repo, enforced, run, scratch};
use crate::LocalComputers;

/// Writes `text` to `path`, making its folder.
fn put(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn someone_else_s_commands_read_none_of_the_person_s_folders_but_their_own_and_toolchains() {
    if !enforced() {
        return;
    }
    let dir = scratch("shut");
    read_by_each(&dir, SandboxLevel::Project, None);
}

/// What the person's command and someone else's read in `dir`, its home folder `dir/home`, at
/// `level`, the sandbox's helper `helper` where it needs one; asserted as the module says.
pub(in crate::local) fn read_by_each(dir: &Path, level: SandboxLevel, helper: Option<&Path>) {
    let (home, projects, app, script) = furnished(dir);
    let environment: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(name, _)| name == "PATH")
        .chain([("HOME".into(), home.clone().into_os_string())])
        .collect();
    let said = block_on!(async {
        let mut computers =
            LocalComputers::new(dir.join("copies"), environment).with_projects(&projects);
        if let Some(helper) = helper {
            computers = computers.with_helper(helper);
        }
        let computer = computers.create(&Environment::host()).await.unwrap();
        let folder = Sources::Folder {
            path: app.to_string_lossy().into_owned(),
        };
        let there = computers.open(computer.id, &folder).await.unwrap().id;
        let owner = Sandbox {
            level,
            network: SandboxNetwork::Off,
            home: Some(home.clone()),
            glave_home: Some(home.join(".glave")),
            runs_for: RunsFor::Owner,
        };
        let someone = Sandbox {
            runs_for: RunsFor::SomeoneElse,
            ..owner.clone()
        };
        let mut said = Vec::new();
        for sandbox in [&owner, &someone] {
            said.push(run(&computers, there, sandbox, &script).await.1);
        }
        said
    });
    checked(&said[0], &said[1]);
}

/// What the person's commands said, `owners`, and someone else's, `theirs`, checked as the
/// module says.
fn checked(owners: &str, theirs: &str) {
    // What the person's commands read, and someone else's neither read nor see listed: on
    // bubblewrap the emptied folders list only the places made in them.
    for read in [
        "the-person-s-notes",
        "another-project-s",
        "a-project-elsewhere",
        "the-cargo-token",
        "the-npm-token",
        "the-dotfile",
        "notes-file.txt",
        "secret-proj",
    ] {
        assert!(owners.contains(read), "the person's own, {read}: {owners}");
        assert!(!theirs.contains(read), "someone else's, {read}: {theirs}");
    }
    let refused = [
        "no-notes",
        "no-other",
        "no-elsewhere",
        "no-cargo-config",
        "no-npmrc",
        "no-dotfiles",
    ];
    for told in refused {
        assert!(!owners.contains(told), "{told}: {owners}");
        assert!(theirs.contains(told), "{told}: {theirs}");
    }
    for works in ["the-toolchain", "the-node", "their-own", "git-works"] {
        assert!(owners.contains(works), "{works}: {owners}");
        assert!(theirs.contains(works), "{works}: {theirs}");
    }
}

/// The person's home folder `dir/home` and projects' folder `dir/projects` with what each keeps,
/// the workspace someone else works in, a repository in the home folder, and the script each
/// runs there.
fn furnished(dir: &Path) -> (PathBuf, PathBuf, PathBuf, String) {
    let home = dir.join("home");
    let projects = dir.join("projects");
    put(&home.join("notes-file.txt"), "the-person-s-notes\n");
    put(
        &home.join("glave/secret-proj/plan.txt"),
        "another-project-s\n",
    );
    put(
        &projects.join("elsewhere/plan.txt"),
        "a-project-elsewhere\n",
    );
    put(&home.join(".cargo/bin/tool"), "the-toolchain\n");
    put(&home.join(".cargo/config.toml"), "the-cargo-token\n");
    let node = home.join(".nvm/versions/node/v1");
    put(&node.join("bin/node"), "the-node\n");
    put(&node.join("etc/npmrc"), "the-npm-token\n");
    put(&home.join("dotfiles/unrelated"), "the-dotfile\n");
    // A dotfiles manager's link, which git's settings include.
    put(
        &home.join("dotfiles/gitlocal"),
        "[alias]\n\tshown = status\n",
    );
    std::os::unix::fs::symlink("dotfiles/gitlocal", home.join(".gitconfig.local")).unwrap();
    put(
        &home.join(".gitconfig"),
        "[include]\n\tpath = ~/.gitconfig.local\n",
    );
    let app = committed_repo(&home.join("glave"), "app");
    put(&app.join("own.txt"), "their-own\n");
    let script = format!(
        "cat \"$HOME/notes-file.txt\" 2>/dev/null || echo no-notes; \
         cat \"$HOME/glave/secret-proj/plan.txt\" 2>/dev/null || echo no-other; \
         cat '{}' 2>/dev/null || echo no-elsewhere; \
         cat \"$HOME/.cargo/config.toml\" 2>/dev/null || echo no-cargo-config; \
         cat '{}' 2>/dev/null || echo no-npmrc; \
         cat \"$HOME/dotfiles/unrelated\" 2>/dev/null || echo no-dotfiles; \
         ls -a \"$HOME\" \"$HOME/glave\" 2>/dev/null; \
         cat \"$HOME/.cargo/bin/tool\" '{}' own.txt; \
         git shown --short >/dev/null && echo git-works",
        projects.join("elsewhere/plan.txt").display(),
        node.join("etc/npmrc").display(),
        node.join("bin/node").display(),
    );
    (home, projects, app, script)
}
