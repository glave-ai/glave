//! Someone else's commands on the person's computer read nothing of the person's home folder or
//! projects' folder but their own workspace, the toolchains there and git's settings (`specs/
//! sandbox.md`, "Whose computer"); the person's own commands read it all.

use std::ffi::OsString;
use std::path::Path;

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
    let home = dir.join("home");
    let projects = dir.join("projects");
    put(&home.join("notes.txt"), "the-person-s-notes\n");
    put(&home.join("glave/other/plan.txt"), "another-project-s\n");
    put(
        &projects.join("elsewhere/plan.txt"),
        "a-project-elsewhere\n",
    );
    put(&home.join(".cargo/bin/tool"), "the-toolchain\n");
    put(&home.join(".gitconfig"), "[core]\n\tautocrlf = false\n");
    let app = committed_repo(&home.join("glave"), "app");
    put(&app.join("own.txt"), "their-own\n");
    let script = format!(
        "cat \"$HOME/notes.txt\" 2>/dev/null || echo no-notes; \
         cat \"$HOME/glave/other/plan.txt\" 2>/dev/null || echo no-other; \
         cat '{}' 2>/dev/null || echo no-elsewhere; \
         ls \"$HOME\" >/dev/null 2>&1 || echo no-listing; \
         cat \"$HOME/.cargo/bin/tool\" own.txt; \
         git status --short >/dev/null && echo git-works",
        projects.join("elsewhere/plan.txt").display()
    );
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
    let (owners, theirs) = (&said[0], &said[1]);
    for read in [
        "the-person-s-notes",
        "another-project-s",
        "a-project-elsewhere",
    ] {
        assert!(owners.contains(read), "the person's own: {owners}");
        assert!(!theirs.contains(read), "someone else's: {theirs}");
    }
    assert!(!owners.contains("no-listing"), "{owners}");
    for told in ["no-notes", "no-other", "no-elsewhere", "no-listing"] {
        assert!(theirs.contains(told), "{told}: {theirs}");
    }
    for works in ["the-toolchain", "their-own", "git-works"] {
        assert!(owners.contains(works), "{works}: {owners}");
        assert!(theirs.contains(works), "{works}: {theirs}");
    }
}
