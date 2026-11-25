//! What someone else's commands on the person's computer never read ([`RunsFor::SomeoneElse`],
//! `specs/sandbox.md`, "Whose computer"): the person's home folder and the folder their
//! projects are made in, hidden whole, but for the toolchains kept there, without which their
//! commands could build and run nothing. git's own settings stay hidden too: their commands'
//! git reads a settings file glave writes instead ([`GIT_SETTINGS`]). The workspace, its
//! repository's shared git folder, the command's own temp folder and what glave lets every
//! sandbox reach stay theirs wherever they are; each platform's sandbox turns this into its own
//! rules.

mod npmrc;

use std::path::{Path, PathBuf};

use glave_ports::computer::{ComputerError, RunsFor, Sandbox};

/// Under the home folder: where toolchains, their version managers and what they install live,
/// as narrowly as each still runs. Not Cargo's home whole (its `config.toml` can hold a
/// registry's token, its `git` and `registry` the source of private crates), nor Go's (its
/// module cache holds private modules): what they install to run alone. The credential stores
/// among them stay hidden (`secrets.rs`), and so does each Node install's own `etc/npmrc`
/// (`npmrc.rs`). A toolchain kept elsewhere in the home folder does not run for someone
/// else.
pub(super) const TOOLCHAINS: &[&str] = &[
    // Rust's.
    ".cargo/bin",
    ".cargo/env",
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
    "go/bin",
    ".dotnet",
    ".juliaup",
    ".ghcup",
    ".elan",
    ".opam",
    ".nix-profile",
];

/// git's settings for someone else's commands, in place of the person's own, which may hold a
/// token or name a file of theirs: the files git would read in the home folder by default
/// named as none, so git looks for nothing there.
pub(crate) const GIT_SETTINGS: &str =
    "[core]\n\texcludesFile = /dev/null\n\tattributesFile = /dev/null\n";

/// The most links followed from one path, as an operating system limits them.
const MOST_LINKS: usize = 40;

/// The person's folders hidden whole from someone else's commands, and what in them stays
/// readable; nothing for the person's own commands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Shut {
    /// Hidden whole, as real paths: the home folder, and the projects' folder where it is not
    /// in it.
    pub(crate) folders: Vec<PathBuf>,
    /// Readable again inside them: the toolchains there and a worktree's shared git folder;
    /// each as named, there or not, every link on the way from it, and where it leads.
    pub(crate) open: Vec<PathBuf>,
    /// Hidden again inside what is readable: each Node install's own `npmrc`.
    pub(crate) hidden: Vec<PathBuf>,
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
    ) -> Result<Self, ComputerError> {
        if sandbox.runs_for == RunsFor::Owner {
            return Ok(Self::default());
        }
        let (home, root) = (sandbox.home.clone(), root.to_owned());
        let (git_common, projects) = (
            git_common.map(Path::to_path_buf),
            projects.map(Path::to_path_buf),
        );
        tokio::task::spawn_blocking(move || {
            Self::reading(home.as_deref(), &root, git_common, projects.as_deref())
        })
        .await
        .map_err(|error| ComputerError::Failed(format!("could not prepare the sandbox: {error}")))
    }

    /// [`Shut::for_sandbox`] for someone else's commands. Blocking: it looks through the Node
    /// managers' folders and follows links.
    fn reading(
        home: Option<&Path>,
        root: &Path,
        git_common: Option<PathBuf>,
        projects: Option<&Path>,
    ) -> Self {
        let home = home.and_then(|home| std::fs::canonicalize(home).ok());
        let projects = projects.and_then(|projects| std::fs::canonicalize(projects).ok());
        let mut folders: Vec<PathBuf> = [home.clone(), projects]
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
        let mut wanted: Vec<PathBuf> = git_common.into_iter().collect();
        let mut found = npmrc::Found::default();
        if let Some(home) = &home {
            found = npmrc::found(home);
            // A Node manager too large to look through for its installs' `npmrc` stays hidden.
            let unsearched = &found.unsearched;
            wanted.extend(
                (TOOLCHAINS.iter())
                    .map(|path| home.join(path))
                    .filter(|path| !unsearched.contains(path)),
            );
        }
        let mut hidden = found.npmrcs;
        let inside = |path: &PathBuf| folders.iter().any(|folder| path.starts_with(folder));
        let mut open: Vec<PathBuf> = wanted
            .iter()
            .flat_map(|path| hops(path))
            .filter(inside)
            .collect();
        open.sort();
        open.dedup();
        hidden.retain(inside);
        Self {
            folders,
            open,
            hidden,
        }
    }

    /// Whether `path` is hidden by it: inside a folder it hides, and not inside what stays
    /// readable there.
    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn hides(&self, path: &Path) -> bool {
        self.folders.iter().any(|folder| path.starts_with(folder))
            && !self.open.iter().any(|open| path.starts_with(open))
    }
}

/// `path` as named, each link it leads through as named, and where it ends, resolved, if it is
/// there: a tool reads each on the way. Blocking.
fn hops(path: &Path) -> Vec<PathBuf> {
    let mut hops = vec![path.to_owned()];
    let mut at = path.to_owned();
    for _ in 0..MOST_LINKS {
        let Ok(target) = std::fs::read_link(&at) else {
            break;
        };
        let next = match at.parent() {
            Some(parent) => parent.join(target),
            None => target,
        };
        // Its folder resolved, so the link itself is named as the system names it.
        let folder = next
            .parent()
            .and_then(|folder| std::fs::canonicalize(folder).ok());
        at = match (folder, next.file_name()) {
            (Some(folder), Some(name)) => folder.join(name),
            _ => next,
        };
        hops.push(at.clone());
    }
    hops.extend(std::fs::canonicalize(path).ok());
    hops
}

#[cfg(test)]
mod tests;
