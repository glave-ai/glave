//! What bubblewrap mounts for one command, read from the workspace as it is just before the
//! command starts: the writable folders, the protected paths inside them laid over read-only,
//! what commands may not read hidden under an empty, unreadable stand-in, and glave's own
//! folder under an empty folder of the sandbox's own, so what glave writes there later stays
//! hidden too.
//!
//! A protected path that does not exist yet gets a stand-in too, an empty folder bubblewrap
//! creates in the project, so a command cannot create it; it is removed once no command needs
//! it any more. Protected folders are mount points while a command runs, so they cannot be
//! renamed or removed, and `.git` itself is one, so it cannot be moved aside for a new one.
//!
//! A place hidden that is gone by the time bubblewrap mounts over it, where bubblewrap can
//! write (`/tmp`), bubblewrap makes again on the host to mount over: an empty folder anyone may
//! enter, or an empty file. tmux and Emacs refuse to start in such a `/tmp/tmux-<uid>` or
//! `/tmp/emacs<uid>`, so once the command ends such a folder is made private, as they make
//! their own, and they start in it (#1539). It is never removed: another command's sandbox may
//! have mounted its own stand-in over it, and removing a mount point on the host takes away
//! every namespace's mount on it, leaving that command to reach what listens there next.

use std::collections::HashMap;
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use glave_ports::computer::{GIT_RUNS_CODE, PROTECTED, SandboxLevel};

use super::super::in_tmp::listeners;
use super::super::secrets::home_secrets;
use crate::local::sandbox::{folders_leading_to, kept_read_only, own_git_folder};

/// Under the user's runtime folder (`$XDG_RUNTIME_DIR`): Podman's sign-ins.
const RUNTIME_SECRETS: &[&str] = &["containers/auth.json"];

/// Where the system's daemons and the person's X server listen: `docker.sock` is root on the
/// machine, Podman's socket the same for its user, and the system bus asks only that its caller
/// be the person at the computer; LXD's, Incus's and libvirt's are root for whoever is in their
/// group, as Ubuntu's first user is in LXD's. And GNU screen's sessions, wherever a system
/// keeps them: a command that reached one would type into a shell outside the sandbox.
const DAEMONS: &[&str] = &[
    "/run/docker.sock",
    "/run/podman",
    "/run/dbus",
    "/run/libvirt",
    "/run/incus",
    "/var/lib/incus",
    "/var/lib/lxd",
    "/var/snap/lxd/common/lxd",
    "/run/screen",
    "/tmp/screens",
    "/tmp/uscreens",
    "/tmp/.X11-unix",
];

/// Inside a git folder, beside what runs code later ([`GIT_RUNS_CODE`]): the folders of other
/// worktrees and of submodules, whose own hooks and config would.
const NESTED_GIT: &[&str] = &["worktrees", "modules"];

/// Inside a worktree's own git folder, beside what runs code later: its submodules' folders.
const WORKTREE_NESTED_GIT: &[&str] = &["modules"];

/// The file naming the folder git takes a repository's config and hooks from, relative to the
/// git folder it is in. git reads it wherever it is there and fails on one it cannot read, so
/// one not there yet gets no empty folder as its stand-in but a file glave writes, naming its
/// own git folder ([`SAME_FOLDER`]), kept there for good: every workspace of a repository
/// protects its shared folder's, and removing one a command's mount lies on would take the
/// mount from that command.
const COMMONDIR: &str = "commondir";

/// What a stand-in `commondir` holds: the git folder it is in.
const SAME_FOLDER: &str = ".\n";

/// `/tmp` as a command sees it.
#[derive(Debug)]
pub(super) enum Tmp {
    /// Shared, which models reach for constantly: this computer's, where it resolves.
    Shared(Option<PathBuf>),
    /// One of each command's own, its own temp folder there: someone else's commands
    /// (`RunsFor::SomeoneElse`), which leave nothing in the shared one that the person's
    /// commands later read or run, and read nothing the person's left there (#1583).
    Own,
}

impl Tmp {
    /// The shared `/tmp`, where commands share one that resolves.
    pub(super) fn shared(&self) -> Option<&PathBuf> {
        match self {
            Self::Shared(tmp) => tmp.as_ref(),
            Self::Own => None,
        }
    }
}

/// Everything a command's mounts depend on, fixed when the sandbox is prepared.
#[derive(Debug)]
pub(super) struct Layout {
    pub(super) root: PathBuf,
    pub(super) level: SandboxLevel,
    pub(super) network: bool,
    /// The folder every glave sandbox keeps read-only.
    pub(super) base: PathBuf,
    pub(super) empty_file: PathBuf,
    pub(super) empty_folder: PathBuf,
    pub(super) tmp: Tmp,
    pub(super) home: Option<PathBuf>,
    /// What the computer hides beside the home folder's credential stores.
    pub(super) also_hidden: Vec<PathBuf>,
    pub(super) glave_home: Option<PathBuf>,
    /// glave's sandbox cache folder, writable, inside glave's own.
    pub(super) cache: Option<PathBuf>,
    /// glave's own folder on commands' `PATH`, readable, never writable, inside glave's own.
    pub(super) glave_on_path: Option<PathBuf>,
    /// The user's runtime folder (`$XDG_RUNTIME_DIR`).
    pub(super) runtime: Option<PathBuf>,
    /// Where the person's daemons listen ([`daemons`]), hidden whole from bubblewrap's commands.
    pub(super) daemons: Vec<PathBuf>,
    /// The sessions' folders the person moved out of `/tmp` ([`moved_sessions`]).
    pub(super) moved_sessions: Vec<PathBuf>,
    /// For a worktree: the repository's shared git folder, where its commits go.
    pub(super) git_common: Option<PathBuf>,
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
    /// The command's own temp folder, at `/tmp` too.
    Tmp(PathBuf),
    /// Hidden whole under an empty folder of the sandbox's own, which the mounts inside it are
    /// made in and which is read-only once they are: what is written there after the command
    /// starts stays hidden, where a stand-in hides only what was there.
    Emptied(PathBuf),
}

impl Mount {
    /// Where it is mounted.
    pub(super) fn path(&self) -> &Path {
        match self {
            Self::Writable(path)
            | Self::ReadOnly(path)
            | Self::HiddenFile(path)
            | Self::HiddenFolder(path)
            | Self::Emptied(path) => path.as_path(),
            Self::Tmp(_) => Path::new("/tmp"),
        }
    }
}

/// Stand-ins in use, by path, with how many running commands use each.
pub(super) type StandIns = Arc<Mutex<HashMap<PathBuf, usize>>>;

/// The stand-ins one command uses; when the last command using one ends, it is removed. And the
/// folders it hides where bubblewrap can write, so that one bubblewrap made again is made private.
#[derive(Debug)]
pub(crate) struct Held {
    paths: Vec<PathBuf>,
    stand_ins: StandIns,
    private: Vec<PathBuf>,
}

/// The mode bubblewrap makes a folder with: `0755`, under its `umask(0)`.
const MADE: u32 = 0o755;

/// Makes the folder at `path` private if it is one bubblewrap made where the private folder
/// hidden there had gone: empty, this user's, and of bubblewrap's mode, which tmux, Emacs and
/// Neovim refuse. Never removed (see the module's docs). Blocking.
fn make_private(path: &Path) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return;
    };
    let mine = meta.uid() == rustix::process::getuid().as_raw();
    if !meta.is_dir() || !mine || meta.mode() & 0o7777 != MADE {
        return;
    }
    let empty = std::fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_none());
    if empty {
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        for path in &self.private {
            make_private(path);
        }
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
/// workspace and the home folder.
pub(super) fn plan(
    layout: &Layout,
    temp: &Path,
    stand_ins: &StandIns,
) -> Result<(Vec<Mount>, Held), String> {
    let mut mounts: Vec<Mount> = (layout.tmp.shared())
        .map(|tmp| Mount::Writable(tmp.clone()))
        .into_iter()
        .collect();
    mounts.push(Mount::ReadOnly(layout.base.clone()));
    mounts.push(Mount::Writable(temp.to_owned()));
    if matches!(layout.tmp, Tmp::Own) {
        mounts.push(Mount::Tmp(temp.to_owned()));
    }
    let mut held = Held {
        paths: Vec::new(),
        stand_ins: Arc::clone(stand_ins),
        private: Vec::new(),
    };
    // glave's own folder needs nothing here: it is emptied wherever the project is (below).
    let kept = kept_read_only(&layout.root, layout.home.as_deref(), None);
    let kept_whole =
        kept.contains(&layout.root) || layout.glave_home.as_ref() == Some(&layout.root);
    match layout.level {
        SandboxLevel::ReadOnly => mounts.push(Mount::ReadOnly(layout.root.clone())),
        SandboxLevel::Project if kept_whole => mounts.push(Mount::ReadOnly(layout.root.clone())),
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
            protect.project(layout, &kept)?;
        }
    }
    mounts.extend(layout.glave_home.iter().cloned().map(Mount::Emptied));
    mounts.extend(layout.cache.iter().cloned().map(Mount::Writable));
    mounts.extend(layout.glave_on_path.iter().cloned().map(Mount::ReadOnly));
    for real in outermost(
        hidden_places(layout)
            .into_iter()
            .chain(daemon_places(layout)?),
    ) {
        held.private.extend(hide(&real, &mut mounts));
    }
    // Only where bubblewrap can write can it make a place again.
    held.private.retain(|path| {
        mounts
            .iter()
            .any(|mount| matches!(mount, Mount::Writable(folder) if path.starts_with(folder)))
    });
    // A folder's mount before those inside it, which it would otherwise lie over: glave's
    // folder holds the cache, and the workspace when it is a thread's worktree there, and is
    // itself inside the workspace when that is the home folder. Stable, so of two mounts at one
    // path the later still wins.
    mounts.sort_by_key(|mount| mount.path().components().count());
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
    fn project(&mut self, layout: &Layout, kept: &[PathBuf]) -> Result<(), String> {
        let git = layout.root.join(".git");
        match self.state(&git)? {
            State::Folder => {
                // Its own mount point, so it cannot be moved aside for a planted one.
                self.mounts.push(Mount::Writable(git.clone()));
                self.inside(&git, NESTED_GIT)?;
            }
            State::File => self.mounts.push(Mount::ReadOnly(git)),
            State::StandIn => self.stand_in(git),
        }
        for path in PROTECTED.iter().filter(|path| !path.starts_with(".git/")) {
            self.protect(layout.root.join(path))?;
        }
        for folder in folders_leading_to(&layout.root, kept).filter(|folder| folder.is_dir()) {
            // Its own mount point, so it cannot be moved with what is kept inside it.
            self.mounts.push(Mount::Writable(folder));
        }
        for path in kept {
            self.protect(path.clone())?;
        }
        if let Some(common) = &layout.git_common {
            self.mounts.push(Mount::Writable(common.clone()));
            self.inside(common, NESTED_GIT)?;
            if let Some(own) = own_git_folder(&layout.root, common) {
                self.mounts.push(Mount::Writable(own.clone()));
                self.inside(&own, WORKTREE_NESTED_GIT)?;
            }
        }
        Ok(())
    }

    /// The git folder `folder`'s paths that run code, and `nested`, its folders of other git
    /// folders.
    fn inside(&mut self, folder: &Path, nested: &[&str]) -> Result<(), String> {
        stand_in_commondir(&folder.join(COMMONDIR))?;
        for name in GIT_RUNS_CODE.iter().chain(nested) {
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

/// Writes `commondir` at `path`, naming its own git folder ([`SAME_FOLDER`]), unless one is
/// there already: the repository's own, or one another sandbox wrote. Never removed
/// ([`COMMONDIR`]). Blocking.
fn stand_in_commondir(path: &Path) -> Result<(), String> {
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut file| file.write_all(SAME_FOLDER.as_bytes()));
    match written {
        Err(error) if error.kind() != std::io::ErrorKind::AlreadyExists => Err(format!(
            "the sandbox cannot protect {}: {error}",
            path.display()
        )),
        _ => Ok(()),
    }
}

/// Where the person's daemons listen, which a command that may make Unix sockets, as one with
/// the network may under bubblewrap, could otherwise reach (`seccomp.rs`): [`DAEMONS`], the
/// ssh agent `agent` names, and the user's runtime folder whole, which holds the session bus (through which systemd starts
/// anything outside the sandbox), the keyring's and gpg's agents, rootless Docker's and
/// Podman's sockets and editors' servers. Hidden whether the network is on or not: one view of
/// the machine. The sessions' folders in `/tmp` are [`sessions`]; what listens in `/tmp` under a
/// name made as it starts is found before each command instead (`in_tmp.rs`).
pub(super) fn daemons(runtime: Option<&Path>, agent: Option<&Path>) -> Vec<PathBuf> {
    DAEMONS
        .iter()
        .map(PathBuf::from)
        .chain(agent.map(Path::to_path_buf))
        .chain(runtime.map(Path::to_path_buf))
        .collect()
}

/// The folders in `tmp` where the user `uid`'s tmux servers, Zellij sessions and Emacs server
/// listen, by names fixed ahead: each runs what it is sent outside the sandbox.
pub(super) fn sessions(tmp: &Path, uid: u32) -> [PathBuf; 3] {
    [tmux(uid), format!("zellij-{uid}"), emacs(uid)].map(|name| tmp.join(name))
}

/// The name of the user `uid`'s tmux servers' folder.
fn tmux(uid: u32) -> String {
    format!("tmux-{uid}")
}

/// The name of the user `uid`'s Emacs server's folder.
fn emacs(uid: u32) -> String {
    format!("emacs{uid}")
}

/// The same folders where the person moved them out of `/tmp`: tmux's to `tmux_in` (its
/// `TMUX_TMPDIR`), Emacs's to `emacs_in` (its `TMPDIR`). Zellij's moves only into the runtime
/// folder, hidden whole already.
pub(super) fn moved_sessions(
    uid: u32,
    tmux_in: Option<&Path>,
    emacs_in: Option<&Path>,
) -> Vec<PathBuf> {
    let moved_tmux = tmux_in.map(|folder| folder.join(tmux(uid)));
    moved_tmux
        .into_iter()
        .chain(emacs_in.map(|folder| folder.join(emacs(uid))))
        .collect()
}

/// `folder`, made first if it is not there, private, as tmux, Zellij and Emacs make their own
/// and start in: hidden only where it is, a session's folder missing when a command's mounts
/// are planned would show the command the server the person starts there while it runs (#1572).
/// Blocking.
fn made_private(folder: PathBuf) -> PathBuf {
    let _ = std::fs::DirBuilder::new().mode(0o700).create(&folder);
    folder
}

/// The real paths of the daemons' places that exist, the sessions' folders in the shared `/tmp`
/// (made if missing), and what listens in `/tmp` for the person's other programs just now
/// (`in_tmp.rs`), except one that holds a place commands use: the workspace, the home folder,
/// `/tmp` or the sandbox's own folders. An error saying why where more listens in `/tmp` than
/// the mounts hide.
fn daemon_places(layout: &Layout) -> Result<Vec<PathBuf>, String> {
    let used: Vec<&Path> = [&layout.root, &layout.base]
        .into_iter()
        .chain(layout.home.iter())
        .chain(layout.tmp.shared())
        .chain(layout.cache.iter())
        .chain(layout.git_common.iter())
        .map(PathBuf::as_path)
        .collect();
    let uid = rustix::process::getuid().as_raw();
    let in_tmp = listeners(Path::new("/tmp"), uid)?;
    let sessions = (layout.tmp.shared().into_iter())
        .flat_map(|tmp| sessions(tmp, uid))
        .chain(layout.moved_sessions.iter().cloned())
        .map(made_private);
    Ok(layout
        .daemons
        .iter()
        .cloned()
        .chain(sessions)
        .chain(in_tmp)
        .filter_map(|place| real_place(&place, uid))
        .filter(|real| !used.iter().any(|used| used.starts_with(real)))
        .collect())
}

/// The real path of `place`, to hide. Where anyone may make names (`/tmp`, any folder others may
/// write), only what this user or the system made there, a link included (an ssh agent's is
/// often one the person put in `/tmp`): another user could put a link at `/tmp/tmux-<uid>` to
/// `/usr` and have every command's sandbox hide it (#1575), but cannot make one that is the
/// person's, nor, in a sticky folder, swap what they own. In such a folder that is not sticky,
/// no link is followed. Elsewhere, where only the system or the person names things, wherever
/// it leads. Blocking.
fn real_place(place: &Path, uid: u32) -> Option<PathBuf> {
    let parent = place
        .parent()
        .and_then(|parent| std::fs::metadata(parent).ok());
    if let Some(parent) = parent.filter(|parent| parent.mode() & 0o002 != 0) {
        let meta = std::fs::symlink_metadata(place).ok()?;
        let owned = meta.uid() == uid || meta.uid() == 0;
        let sticky = parent.mode() & 0o1000 != 0;
        if !owned || (meta.file_type().is_symlink() && !sticky) {
            return None;
        }
    }
    std::fs::canonicalize(place).ok()
}

/// `places` without those inside another of them: hiding the outer one hides them already, and
/// no stand-in could be made inside its stand-in.
fn outermost(places: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut places: Vec<PathBuf> = places.into_iter().collect();
    places.sort();
    places.dedup();
    let mut kept: Vec<PathBuf> = Vec::new();
    for place in places {
        if !kept.iter().any(|outer| place.starts_with(outer)) {
            kept.push(place);
        }
    }
    kept
}

/// The real paths of the home folder's credential folders and files, and of whatever else the
/// computer hides, that exist, except one that holds the workspace. Blocking.
pub(super) fn hidden_places(layout: &Layout) -> Vec<PathBuf> {
    let in_home = layout
        .home
        .iter()
        .flat_map(|home| home_secrets().map(move |secret| home.join(secret)));
    let in_runtime = layout.runtime.iter().flat_map(|runtime| {
        RUNTIME_SECRETS
            .iter()
            .map(move |secret| runtime.join(secret))
    });
    // Someone else's commands read nothing the person's left in `/var/tmp` either (#1583).
    let var_tmp = matches!(layout.tmp, Tmp::Own).then(|| PathBuf::from("/var/tmp"));
    in_home
        .chain(in_runtime)
        .chain(layout.also_hidden.iter().cloned())
        .chain(var_tmp)
        .filter_map(|secret| std::fs::canonicalize(secret).ok())
        .filter(|real| !layout.root.starts_with(real))
        .collect()
}

/// `path` hidden under the stand-in of its kind, and `path` again if it is a private folder,
/// which bubblewrap would make again as one that is not; links are left, their targets judged
/// alone.
fn hide(path: &Path, mounts: &mut Vec<Mount>) -> Option<PathBuf> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if meta.file_type().is_symlink() {
        return None;
    }
    if !meta.is_dir() {
        mounts.push(Mount::HiddenFile(path.to_owned()));
        return None;
    }
    mounts.push(Mount::HiddenFolder(path.to_owned()));
    // No permission bit for its group or others.
    (meta.mode().trailing_zeros() >= 6).then(|| path.to_owned())
}

#[cfg(test)]
mod tests {
    use glave_ports::testing::TempDir;

    use super::*;

    /// The folders a test plans in, `tmp`, `project` and `temp`, under the real path returned;
    /// `tmp` anyone's to write, as `/tmp` is.
    fn folders() -> (TempDir, PathBuf) {
        let dir = TempDir::new("sessions").unwrap();
        let real = std::fs::canonicalize(dir.path()).unwrap();
        for folder in ["tmp", "project", "temp"] {
            std::fs::create_dir(real.join(folder)).unwrap();
        }
        let shared = std::fs::Permissions::from_mode(0o1777);
        std::fs::set_permissions(real.join("tmp"), shared).unwrap();
        (dir, real)
    }

    /// The mounts of a command planned in `dir`'s folders, `tmp` as the shared `/tmp`, `moved`
    /// as the sessions' folders moved out of it and `daemons` as where daemons listen.
    fn planned(dir: &Path, moved: Vec<PathBuf>, daemons: Vec<PathBuf>) -> Vec<Mount> {
        let layout = Layout {
            root: dir.join("project"),
            level: SandboxLevel::Project,
            network: true,
            base: dir.join("base"),
            empty_file: dir.join("base/empty"),
            empty_folder: dir.join("base/empty.d"),
            tmp: Tmp::Shared(Some(dir.join("tmp"))),
            home: None,
            also_hidden: Vec::new(),
            glave_home: None,
            cache: None,
            glave_on_path: None,
            runtime: None,
            daemons,
            moved_sessions: moved,
            git_common: None,
        };
        let (mounts, _held) = plan(&layout, &dir.join("temp"), &StandIns::default()).unwrap();
        mounts
    }

    /// Whether `folder` is made, private as tmux makes its own, and hidden in `mounts`.
    fn hidden_and_private(folder: &Path, mounts: &[Mount]) {
        let shown = folder.display();
        let mode = std::fs::metadata(folder).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "{shown}");
        let hidden = Mount::HiddenFolder(folder.to_owned());
        assert!(mounts.contains(&hidden), "{shown}: {mounts:?}");
    }

    /// A session's folder missing from the shared `/tmp` when a command's mounts are planned is
    /// made, private as tmux makes its own, and hidden like one that was there: a server the
    /// person starts in it while the command runs is out of the command's reach (#1572).
    #[test]
    fn a_session_s_folder_missing_from_tmp_is_made_private_and_hidden() {
        let (_dir, real) = folders();
        let mounts = planned(&real, Vec::new(), Vec::new());
        let uid = rustix::process::getuid().as_raw();
        for folder in sessions(&real.join("tmp"), uid) {
            hidden_and_private(&folder, &mounts);
        }
    }

    /// tmux's folder moved by `TMUX_TMPDIR`, and Emacs's by `TMPDIR`, are made, private, and
    /// hidden there as those in `/tmp` are (#1575).
    #[test]
    fn a_session_s_folder_moved_out_of_tmp_is_hidden_there() {
        let (_dir, real) = folders();
        let uid = rustix::process::getuid().as_raw();
        let (tmux_in, emacs_in) = (real.join("project"), real.join("temp"));
        let moved = moved_sessions(uid, Some(&tmux_in), Some(&emacs_in));
        assert_eq!(moved.len(), 2);
        let mounts = planned(&real, moved.clone(), Vec::new());
        for folder in &moved {
            hidden_and_private(folder, &mounts);
        }
    }

    /// A link the person put in a shared, sticky folder is followed, and what it leads to
    /// hidden: their ssh agent's, `/tmp/ssh-agent-<user>`, which another user could not make
    /// nor swap (#1575).
    #[test]
    fn the_person_s_link_to_their_ssh_agent_in_tmp_is_followed_and_hidden() {
        let (dir, real) = folders();
        let agent = glave_ports::testing::SocketPath::new(&dir, "agent.sock").unwrap();
        let _listening = std::os::unix::net::UnixListener::bind(&agent).unwrap();
        let link = real.join("tmp/ssh-agent-glave");
        std::os::unix::fs::symlink(&agent, &link).unwrap();
        let mounts = planned(&real, Vec::new(), vec![link]);
        let socket = std::fs::canonicalize(&agent).unwrap();
        assert!(mounts.contains(&Mount::HiddenFile(socket)), "{mounts:?}");
    }

    /// In a folder anyone may write that is not sticky, where another user could swap a link for
    /// one of theirs, no link is followed: what it leads to is left as it is.
    #[test]
    fn a_link_in_a_shared_folder_that_is_not_sticky_is_never_followed() {
        let (_dir, real) = folders();
        let open = std::fs::Permissions::from_mode(0o777);
        std::fs::set_permissions(real.join("tmp"), open).unwrap();
        let system = real.join("usr");
        std::fs::create_dir(&system).unwrap();
        let uid = rustix::process::getuid().as_raw();
        let [tmux, ..] = sessions(&real.join("tmp"), uid);
        std::os::unix::fs::symlink(&system, &tmux).unwrap();
        let mounts = planned(&real, Vec::new(), Vec::new());
        let reached = |path: &Path| mounts.iter().any(|mount| mount.path() == path);
        assert!(!reached(&system), "{mounts:?}");
        assert!(!reached(&tmux), "{mounts:?}");
    }
}
