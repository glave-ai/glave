use glave_ports::block_on;
use glave_ports::computer::{SandboxLevel, SandboxNetwork};
use glave_ports::testing::TempDir;

use super::*;

fn sandbox(home: &Path, runs_for: RunsFor) -> Sandbox {
    Sandbox {
        level: SandboxLevel::Project,
        network: SandboxNetwork::Off,
        home: Some(home.to_owned()),
        glave_home: Some(home.join(".glave")),
        runs_for,
    }
}

#[test]
fn someone_else_s_commands_lose_the_home_and_projects_folders_but_toolchains() {
    let dir = TempDir::new("shut").unwrap();
    let real = std::fs::canonicalize(dir.path()).unwrap();
    let home = real.join("home");
    for folder in [
        "home/.cargo/bin",
        "home/glave/app",
        "home/notes",
        "home/dotfiles/tools",
        "home/.config/git",
        "home/.local",
        "projects/other",
    ] {
        std::fs::create_dir_all(real.join(folder)).unwrap();
    }
    // A dotfiles manager's link.
    std::os::unix::fs::symlink("../dotfiles/tools", home.join(".local/bin")).unwrap();
    let root = home.join("glave/app");
    let projects = real.join("projects");
    block_on!(async {
        let owner = sandbox(&home, RunsFor::Owner);
        let none = Shut::for_sandbox(&owner, &root, None, Some(&projects));
        let none = none.await.unwrap();
        assert_eq!(none, Shut::default(), "the person's own hide nothing more");

        let someone = sandbox(&home, RunsFor::SomeoneElse);
        let shut = Shut::for_sandbox(&someone, &root, None, Some(&projects));
        let shut = shut.await.unwrap();
        assert_eq!(shut.folders, vec![home.clone(), projects.clone()]);
        for open in [".cargo/bin", ".local/bin", "dotfiles/tools"] {
            let path = home.join(open);
            assert!(shut.open.contains(&path), "{open}: {:?}", shut.open);
        }
        assert!(shut.hides(&home.join("notes")));
        assert!(shut.hides(&home.join(".cargo/config.toml")));
        assert!(shut.hides(&home.join("dotfiles/other")));
        assert!(
            shut.hides(&home.join(".gitconfig")),
            "git's settings are the person's"
        );
        assert!(shut.hides(&home.join(".config/git/config")));
        assert!(shut.hides(&projects.join("other")));
        assert!(!shut.hides(&home.join(".cargo/bin/cargo")));
        assert!(!shut.hides(&real.join("elsewhere")));

        let base = home.join("glave");
        let in_home = Shut::for_sandbox(&someone, &root, None, Some(&base));
        let in_home = in_home.await.unwrap();
        assert_eq!(in_home.folders, vec![home.clone()], "one inside another");

        let at_home = Shut::for_sandbox(&someone, &home, None, Some(&projects));
        let at_home = at_home.await.unwrap();
        assert_eq!(at_home.folders, vec![projects], "the workspace is its own");
    });
}
