//! What someone else's commands on the person's computer never read ([`RunsFor::SomeoneElse`],
//! `specs/sandbox.md`, "Whose computer"): the person's home folder and the folder their
//! projects are made in, hidden whole, but for the toolchains kept there and git's own
//! settings, without which their commands could build and run nothing. The workspace, its
//! repository's shared git folder, the command's own temp folder and what glave lets every
//! sandbox reach stay theirs wherever they are; each platform's sandbox turns this into its own
//! rules.

use std::path::{Path, PathBuf};

use glave_ports::computer::{RunsFor, Sandbox};

/// Under the home folder: where toolchains, their version managers and what they install live,
/// and git's settings, which git refuses to run without once it may not read them. The
/// credential stores among them stay hidden (`secrets.rs`).
pub(super) const TOOLCHAINS: &[&str] = &[
    // Rust's.
    ".cargo",
    ".rustup",
    // What tools install for the person to run by name.
    ".local/bin",
    // Python's, Node's, and the version managers that keep many languages'.
    ".local/share/uv",
    ".pyenv",
    ".nvm",
    ".fnm",
    ".local/share/fnm",
    "Library/Application Support/fnm",
    ".volta",
    ".local/share/pnpm",
    "Library/pnpm",
    ".bun",
    ".deno",
    ".local/share/mise",
    ".asdf",
    ".rbenv",
    ".rvm",
    ".sdkman",
    ".jenv",
    ".goenv",
    "go",
    ".dotnet",
    ".juliaup",
    ".ghcup",
    ".elan",
    ".opam",
    ".nix-profile",
    // git's own settings.
    ".gitconfig",
    ".config/git",
];

/// The person's folders hidden whole from someone else's commands, and what in them stays
/// readable, all as real paths; nothing for the person's own commands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Shut {
    /// Hidden whole: the home folder, and the projects' folder where it is not in it.
    pub(crate) folders: Vec<PathBuf>,
    /// Readable again inside them: the toolchains there, and a worktree's shared git folder,
    /// each as named, there or not, and where a link leads.
    pub(crate) open: Vec<PathBuf>,
}

impl Shut {
    /// What `sandbox` hides of the person's for commands in `root`, a worktree's repository's
    /// shared git folder at `git_common`, on a computer that makes projects' folders in
    /// `projects`. A folder that is the workspace or inside it is never hidden: it is the
    /// workspace's.
    pub(crate) async fn for_sandbox(
        sandbox: &Sandbox,
        root: &Path,
        git_common: Option<&Path>,
        projects: Option<&Path>,
    ) -> Self {
        if sandbox.runs_for == RunsFor::Owner {
            return Self::default();
        }
        let home = real(sandbox.home.as_deref()).await;
        let mut folders: Vec<PathBuf> = [home.clone(), real(projects).await]
            .into_iter()
            .flatten()
            .filter(|folder| !folder.starts_with(root))
            .collect();
        folders.sort();
        folders.dedup();
        let outer = folders.clone();
        folders.retain(|folder| {
            !outer
                .iter()
                .any(|other| other != folder && folder.starts_with(other))
        });
        let mut wanted: Vec<PathBuf> = home
            .iter()
            .flat_map(|home| TOOLCHAINS.iter().map(move |path| home.join(path)))
            .collect();
        wanted.extend(git_common.map(Path::to_path_buf));
        // Each as named, there yet or not, so a tool that looks for it is told it is not there
        // rather than refused (git stops at a settings file it may not look for), and where a
        // link leads.
        let mut open = Vec::new();
        for path in wanted {
            let real = real(Some(&path)).await;
            for path in [Some(path), real].into_iter().flatten() {
                if folders.iter().any(|folder| path.starts_with(folder)) {
                    open.push(path);
                }
            }
        }
        open.sort();
        open.dedup();
        Self { folders, open }
    }

    /// Whether `path` is hidden by it: inside a folder it hides, and not inside what stays
    /// readable there.
    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn hides(&self, path: &Path) -> bool {
        self.folders.iter().any(|folder| path.starts_with(folder))
            && !self.open.iter().any(|open| path.starts_with(open))
    }
}

/// `path` resolved, if it is there.
async fn real(path: Option<&Path>) -> Option<PathBuf> {
    tokio::fs::canonicalize(path?).await.ok()
}

#[cfg(test)]
mod tests {
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
            "projects/other",
        ] {
            std::fs::create_dir_all(real.join(folder)).unwrap();
        }
        std::fs::write(home.join(".gitconfig"), "").unwrap();
        let root = home.join("glave/app");
        let projects = real.join("projects");
        block_on!(async {
            let owner = sandbox(&home, RunsFor::Owner);
            let none = Shut::for_sandbox(&owner, &root, None, Some(&projects)).await;
            assert_eq!(none, Shut::default(), "the person's own hide nothing more");

            let someone = sandbox(&home, RunsFor::SomeoneElse);
            let shut = Shut::for_sandbox(&someone, &root, None, Some(&projects)).await;
            assert_eq!(shut.folders, vec![home.clone(), projects.clone()]);
            for open in [".cargo", ".gitconfig", ".config/git"] {
                assert!(shut.open.contains(&home.join(open)), "{open}");
            }
            assert!(shut.hides(&home.join("notes")));
            assert!(shut.hides(&projects.join("other")));
            assert!(!shut.hides(&home.join(".cargo/bin")));
            assert!(!shut.hides(&real.join("elsewhere")));

            let base = home.join("glave");
            let in_home = Shut::for_sandbox(&someone, &root, None, Some(&base));
            assert_eq!(
                in_home.await.folders,
                vec![home.clone()],
                "one inside another"
            );

            let at_home = Shut::for_sandbox(&someone, &home, None, Some(&projects)).await;
            assert_eq!(at_home.folders, vec![projects], "the workspace is its own");
        });
    }
}
