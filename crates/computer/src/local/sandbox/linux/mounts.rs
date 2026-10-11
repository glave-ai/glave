//! What bubblewrap mounts for one command, read from the workspace as it is just before the
//! command starts: the writable folders, the protected paths inside them laid over read-only,
//! and what commands may not read hidden under an empty, unreadable stand-in.
//!
//! A protected path that does not exist yet gets a stand-in too, an empty folder bubblewrap
//! creates in the project, so a command cannot create it; it is removed once no command needs
//! it any more. Protected folders are mount points while a command runs, so they cannot be
//! renamed or removed, and `.git` itself is one, so it cannot be moved aside for a new one.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use glave_ports::{PROTECTED, SandboxLevel};

use super::names::SecretNames;
use crate::local::sandbox::own_git_folder;

/// Under the home folder: where credentials live. Never readable from the sandbox.
const HOME_SECRETS: &[&str] = &[
    ".ssh",
    ".aws",
    ".azure",
    ".config/gcloud",
    ".kube",
    ".docker/config.json",
    ".gnupg",
    ".netrc",
    ".npmrc",
    ".pypirc",
    ".git-credentials",
    ".local/share/keyrings",
    ".password-store",
    ".mozilla",
    ".config/google-chrome",
    ".config/chromium",
    ".config/BraveSoftware",
    ".config/microsoft-edge",
];

/// Inside a git folder: what runs code later, and the folders of other worktrees and of
/// submodules, whose own hooks and config would.
const GIT_PROTECTED: &[&str] = &["hooks", "config", "config.worktree", "worktrees", "modules"];

/// Inside a worktree's own git folder: what runs code later, and its submodules' folders.
const WORKTREE_PROTECTED: &[&str] = &["hooks", "config", "config.worktree", "modules"];

/// Everything a command's mounts depend on, fixed when the sandbox is prepared.
#[derive(Debug)]
pub(super) struct Layout {
    pub(super) root: PathBuf,
    pub(super) level: SandboxLevel,
    pub(super) network: bool,
    /// The run's temp folder (`TMPDIR`), inside `base`.
    pub(super) temp: PathBuf,
    /// The folder every glave sandbox keeps read-only.
    pub(super) base: PathBuf,
    pub(super) empty_file: PathBuf,
    pub(super) empty_folder: PathBuf,
    /// `/tmp`, shared, which models reach for constantly.
    pub(super) shared_tmp: Option<PathBuf>,
    pub(super) home: Option<PathBuf>,
    pub(super) glave_home: Option<PathBuf>,
    /// For a worktree: the repository's shared git folder, where its commits go.
    pub(super) git_common: Option<PathBuf>,
    pub(super) names: SecretNames,
}

/// One mount, in the order bubblewrap applies them: later ones lie over earlier ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Mount {
    Writable(PathBuf),
    ReadOnly(PathBuf),
    /// Hidden under the empty file.
    HiddenFile(PathBuf),
    /// Hidden under the empty folder; also the stand-in for a protected path not there yet.
    HiddenFolder(PathBuf),
}

/// Stand-ins in use, by path, with how many running commands use each.
pub(super) type StandIns = Arc<Mutex<HashMap<PathBuf, usize>>>;

/// The stand-ins one command uses; when the last command using one ends, it is removed.
#[derive(Debug)]
pub(crate) struct Held {
    paths: Vec<PathBuf>,
    stand_ins: StandIns,
}

impl Drop for Held {
    fn drop(&mut self) {
        let Ok(mut counts) = self.stand_ins.lock() else {
            return;
        };
        for path in &self.paths {
            let left = counts.get(path).map_or(0, |count| count.saturating_sub(1));
            if left > 0 {
                counts.insert(path.clone(), left);
                continue;
            }
            counts.remove(path);
            // Only the empty folder bubblewrap made: anything else there is someone's.
            if std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir()) {
                let _ = std::fs::remove_dir(path);
            }
        }
    }
}

/// The mounts for the next command, and the stand-ins it holds. Blocking: it reads the
/// workspace, the home folder and glave's own folder.
pub(super) fn plan(layout: &Layout, stand_ins: &StandIns) -> Result<(Vec<Mount>, Held), String> {
    let secret_files = secret_files(&layout.root, &layout.names)
        .map_err(|error| format!("could not read {}: {error}", layout.root.display()))?;
    let mut mounts: Vec<Mount> = layout
        .shared_tmp
        .iter()
        .map(|tmp| Mount::Writable(tmp.clone()))
        .collect();
    mounts.push(Mount::ReadOnly(layout.base.clone()));
    mounts.push(Mount::Writable(layout.temp.clone()));
    let mut held = Held {
        paths: Vec::new(),
        stand_ins: Arc::clone(stand_ins),
    };
    match layout.level {
        SandboxLevel::ReadOnly => mounts.push(Mount::ReadOnly(layout.root.clone())),
        SandboxLevel::Project => {
            mounts.push(Mount::Writable(layout.root.clone()));
            let mut counts = stand_ins
                .lock()
                .map_err(|_| "the sandbox's lock is poisoned".to_owned())?;
            let mut protect = Protect {
                mounts: &mut mounts,
                counts: &mut counts,
                held: &mut held.paths,
            };
            protect.project(layout)?;
        }
    }
    hide_glave_home(layout, &mut mounts);
    hide_home_secrets(layout, &mut mounts);
    mounts.extend(secret_files.into_iter().map(Mount::HiddenFile));
    Ok((mounts, held))
}

/// What a protected path is now.
enum State {
    /// Not there, or a stand-in another command made: it gets a stand-in.
    StandIn,
    Folder,
    File,
}

/// Protected paths inside the writable project, laid over read-only.
struct Protect<'a> {
    mounts: &'a mut Vec<Mount>,
    counts: &'a mut HashMap<PathBuf, usize>,
    held: &'a mut Vec<PathBuf>,
}

impl Protect<'_> {
    fn project(&mut self, layout: &Layout) -> Result<(), String> {
        let git = layout.root.join(".git");
        match self.state(&git)? {
            State::Folder => {
                // Its own mount point, so it cannot be moved aside for a planted one.
                self.mounts.push(Mount::Writable(git.clone()));
                self.inside(&git, GIT_PROTECTED)?;
            }
            State::File => self.mounts.push(Mount::ReadOnly(git)),
            State::StandIn => self.stand_in(git),
        }
        for path in PROTECTED.iter().filter(|path| !path.starts_with(".git/")) {
            self.protect(layout.root.join(path))?;
        }
        if let Some(common) = &layout.git_common {
            self.mounts.push(Mount::Writable(common.clone()));
            self.inside(common, GIT_PROTECTED)?;
            if let Some(own) = own_git_folder(&layout.root, common) {
                self.mounts.push(Mount::Writable(own.clone()));
                self.inside(&own, WORKTREE_PROTECTED)?;
            }
        }
        Ok(())
    }

    fn inside(&mut self, folder: &Path, names: &[&str]) -> Result<(), String> {
        for name in names {
            self.protect(folder.join(name))?;
        }
        Ok(())
    }

    fn protect(&mut self, path: PathBuf) -> Result<(), String> {
        match self.state(&path)? {
            State::Folder | State::File => self.mounts.push(Mount::ReadOnly(path)),
            State::StandIn => self.stand_in(path),
        }
        Ok(())
    }

    fn stand_in(&mut self, path: PathBuf) {
        *self.counts.entry(path.clone()).or_default() += 1;
        self.mounts.push(Mount::HiddenFolder(path.clone()));
        self.held.push(path);
    }

    fn state(&self, path: &Path) -> Result<State, String> {
        if self.counts.contains_key(path) {
            return Ok(State::StandIn);
        }
        match std::fs::symlink_metadata(path) {
            Err(_) => Ok(State::StandIn),
            Ok(meta) if meta.file_type().is_symlink() => Err(format!(
                "the sandbox cannot protect {}: it is a symbolic link, and what it points to \
                 could be swapped; make it a real file or folder, or run without the sandbox",
                path.display()
            )),
            Ok(meta) if meta.is_dir() => Ok(State::Folder),
            Ok(_) => Ok(State::File),
        }
    }
}

/// glave's own folder, all of it, except the way to the workspace when it lives there.
fn hide_glave_home(layout: &Layout, mounts: &mut Vec<Mount>) {
    let Some(glave_home) = &layout.glave_home else {
        return;
    };
    let Ok(inside) = layout.root.strip_prefix(glave_home) else {
        hide(glave_home, mounts);
        return;
    };
    let mut folder = glave_home.clone();
    for part in inside.components() {
        let next = folder.join(part);
        for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
            if entry.path() != next {
                hide(&entry.path(), mounts);
            }
        }
        folder = next;
    }
}

/// Credential folders and files in the home folder, wherever their links lead.
fn hide_home_secrets(layout: &Layout, mounts: &mut Vec<Mount>) {
    for real in home_secrets(layout) {
        hide(&real, mounts);
    }
}

/// The real paths of the home folder's credential folders and files that exist, except one
/// that holds the workspace.
fn home_secrets(layout: &Layout) -> Vec<PathBuf> {
    let Some(home) = &layout.home else {
        return Vec::new();
    };
    HOME_SECRETS
        .iter()
        .filter_map(|secret| std::fs::canonicalize(home.join(secret)).ok())
        .filter(|real| !layout.root.starts_with(real))
        .collect()
}

/// What commands may not read, for Landlock: the home folder's credentials and the
/// project's files named like secrets. Blocking.
pub(super) fn hidden(layout: &Layout) -> Result<Vec<PathBuf>, String> {
    let mut hidden = home_secrets(layout);
    hidden.extend(
        secret_files(&layout.root, &layout.names)
            .map_err(|error| format!("could not read {}: {error}", layout.root.display()))?,
    );
    Ok(hidden)
}

/// `path` hidden under the stand-in of its kind; links are left, their targets judged alone.
fn hide(path: &Path, mounts: &mut Vec<Mount>) {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {}
        Ok(meta) if meta.is_dir() => mounts.push(Mount::HiddenFolder(path.to_owned())),
        Ok(_) => mounts.push(Mount::HiddenFile(path.to_owned())),
        Err(_) => {}
    }
}

/// Files in the project named like secrets, outside `.git` folders, links not followed.
fn secret_files(root: &Path, names: &SecretNames) -> std::io::Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut folders = vec![root.to_owned()];
    let mut first = true;
    while let Some(folder) = folders.pop() {
        let entries = match std::fs::read_dir(&folder) {
            Ok(entries) => entries,
            // A folder glave cannot read, commands cannot read either.
            Err(_) if !first => continue,
            Err(error) => return Err(error),
        };
        first = false;
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() && entry.file_name() != ".git" {
                folders.push(entry.path());
            } else if kind.is_file() && names.is_secret(&entry.file_name().to_string_lossy()) {
                found.push(entry.path());
            }
        }
    }
    Ok(found)
}
