//! The sandbox around commands on this machine (`specs/sandbox.md`): the operating system
//! refuses writes outside the workspace, reads of secret locations and, unless allowed, the
//! network, whatever a command tries. macOS uses Seatbelt; Linux uses bubblewrap with a
//! seccomp filter, or Landlock with the same filter at the `read-only` level where bubblewrap
//! cannot start. Elsewhere a sandboxed command is refused, never run unconfined.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod outbound;
#[cfg(any(target_os = "macos", test))]
mod profile;
mod run_temp;
#[cfg(any(target_os = "macos", target_os = "linux", test))]
mod secrets;
#[cfg(test)]
mod tests;
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod unsupported;

#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
use unsupported as platform;

/// glave's own program as Linux's helpers: the sandbox's, the forwarder, and the one that
/// joins a thread's network.
#[cfg(target_os = "linux")]
use linux::{enter, forward, helper};
#[cfg(not(target_os = "linux"))]
use only_on_linux::{enter, forward, helper};

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use glave_ports::computer::{ComputerError, Sandbox, git_settings};
use glave_process::Processes;

use run_temp::RunTemp;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) use outbound::Outbound;

/// How a sandbox's outbound proxy is made: nothing, where there is no sandbox to keep commands
/// to one (`specs/sandbox.md`, "The outbound proxy").
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
#[derive(Debug, Clone, Default)]
pub(crate) struct Outbound {}

/// The first argument that makes glave the Linux sandbox's helper (see [`sandbox_helper`]).
pub(crate) const SANDBOX_HELPER_ARG: &str = "__glave-sandbox";

/// Runs this process as one of the outbound proxy's helpers on Linux when `args` (the whole
/// command line) are one's: `glave __glave-forward <socket>` holds a thread's network
/// namespace, carrying the connections made to a port of its loopback to the outbound proxy's
/// socket until it is stopped; `glave __glave-enter <pid> <user> <network> -- <command>…`
/// joins that namespace, then becomes the command. Returns `None` when they are not, and
/// otherwise only when either failed, saying what went wrong. A program that passes its own
/// path to [`crate::LocalComputers::with_helper`] calls this first thing in `main`, before it
/// starts any threads: a process with more than one cannot join a user namespace.
pub fn forward_helper(args: &[OsString]) -> Option<String> {
    let rest = args.get(2..).unwrap_or_default();
    match args.get(1) {
        Some(first) if first == FORWARD_HELPER_ARG => Some(forward(rest)),
        Some(first) if first == ENTER_HELPER_ARG => Some(enter(rest)),
        _ => None,
    }
}

/// Linux's helpers where there is no Linux: each says so.
#[cfg(not(target_os = "linux"))]
mod only_on_linux {
    use std::ffi::OsString;

    const ONLY_ON_LINUX: &str = "this helper of glave's runs only on Linux";

    pub(super) fn helper(_args: &[OsString]) -> String {
        ONLY_ON_LINUX.into()
    }

    pub(super) fn forward(_args: &[OsString]) -> String {
        ONLY_ON_LINUX.into()
    }

    pub(super) fn enter(_args: &[OsString]) -> String {
        ONLY_ON_LINUX.into()
    }
}

/// The first argument that makes glave the forwarder holding a thread's network namespace
/// (see [`forward_helper`]).
pub(crate) const FORWARD_HELPER_ARG: &str = "__glave-forward";

/// The first argument that makes glave join a thread's network namespace for one command
/// (see [`forward_helper`]).
pub(crate) const ENTER_HELPER_ARG: &str = "__glave-enter";

/// Whether this machine can enforce every sandbox level: Seatbelt on macOS, bubblewrap on
/// Linux. (Landlock, Linux's fallback, enforces only `read-only`, and only for a program that
/// names its helper.)
pub fn sandbox_available() -> bool {
    platform::available()
}

/// Runs this process as the Linux sandbox's helper when `args` (the whole command line) are
/// the helper's: `glave __glave-sandbox <plan> -- <command>…` confines itself with Landlock
/// and seccomp, then becomes the command. Returns `None` when they are not, and otherwise only
/// when confining or starting the command failed, saying what went wrong. A program that
/// passes its own path to [`crate::LocalComputers::with_helper`] calls this first
/// thing in `main`, before it starts any threads.
pub fn sandbox_helper(args: &[OsString]) -> Option<String> {
    if args.get(1).is_none_or(|first| first != SANDBOX_HELPER_ARG) {
        return None;
    }
    Some(helper(args.get(2..).unwrap_or_default()))
}

/// A sandbox ready to wrap commands in one workspace.
#[derive(Debug)]
pub(crate) struct Prepared {
    /// What it was prepared from, so a command asking for another sandbox gets its own.
    pub(crate) sandbox: Sandbox,
    temps: Temps,
    /// glave's sandbox cache folder, when there is glave's own folder.
    cache: Option<PathBuf>,
    enforcer: platform::Enforcer,
}

/// What of glave's own folder, which they may not otherwise read, sandboxed commands still
/// reach.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Reached<'a> {
    /// glave's sandbox cache folder, readable and writable.
    pub(crate) cache: Option<&'a Path>,
    /// glave's folder on commands' `PATH`, readable, never writable.
    pub(crate) glave_on_path: Option<&'a Path>,
}

/// Where each command's own temp folder is made, and what its name starts with; and where
/// every glave's folders that hold them sit, `<prefix><pid>-<n>` named for their glave.
#[derive(Debug)]
struct Temps {
    parent: PathBuf,
    prefix: &'static str,
    every: PathBuf,
    every_prefix: &'static str,
}

/// How to start one command in the sandbox: a program and the arguments before the command.
#[derive(Debug)]
pub(crate) struct Launch {
    pub(crate) program: PathBuf,
    pub(crate) args: Vec<OsString>,
    /// Variables the wrapper is given besides the command's, and which win over them: the
    /// outbound proxy's URL, with its secret, for a command kept to it, at the port its thread's
    /// forwarder listens on in the thread's namespace on Linux, at the proxy's own on macOS.
    pub(crate) env: Vec<(OsString, OsString)>,
    /// Bytes for the wrapper's standard input, which is then closed: Linux's seccomp filter.
    pub(crate) input: Option<Vec<u8>>,
    /// The wrapper starts a session of its own, so the command must not get a process group
    /// first (a group leader cannot start a session).
    pub(crate) own_session: bool,
    /// The command's own temp folder, its `TMPDIR`, removed once the command has ended.
    pub(crate) temp: RunTemp,
    /// What launching left in the workspace, removed once the command has ended.
    pub(crate) _held: platform::Held,
}

impl Prepared {
    /// Prepares `sandbox` for every command run in `root`. `git_common`, for a worktree, is
    /// its repository's shared git folder, writable too. `helper` is a program that runs
    /// [`sandbox_helper`], which Linux needs where bubblewrap cannot start, and
    /// [`forward_helper`], which on Linux holds the network namespace of commands kept to the
    /// outbound proxy `outbound` makes, forwarding to it, started through `processes` (on macOS
    /// they reach the proxy directly). `hidden` is what
    /// this computer hides beside the home folder's credential stores. `glave_on_path`, glave's
    /// own folder on commands' `PATH`, stays readable, never writable, though it sits in
    /// glave's folder.
    pub(crate) async fn new(
        sandbox: &Sandbox,
        root: &Path,
        git_common: Option<&Path>,
        helpers: (Option<&Path>, &Outbound, &Processes),
        hidden: &[PathBuf],
        glave_on_path: Option<&Path>,
    ) -> Result<Self, ComputerError> {
        let cache = cache_home(sandbox.glave_home.as_deref()).await;
        let reached = Reached {
            cache: cache.as_deref(),
            glave_on_path,
        };
        let (enforcer, temps) =
            platform::Enforcer::prepare(sandbox, root, git_common, helpers, reached, hidden)
                .await?;
        // What a glave killed outright left: it could not remove its own.
        let (every, prefix) = (temps.every.clone(), temps.every_prefix);
        let _ = tokio::task::spawn_blocking(move || run_temp::remove_left(&every, prefix)).await;
        Ok(Self {
            sandbox: sandbox.clone(),
            temps,
            cache,
            enforcer,
        })
    }

    /// Where package managers keep their caches for `launch`'s command: glave's sandbox
    /// cache folder, kept between runs, or without glave's folder one in its temp folder.
    pub(crate) fn caches(&self, launch: &Launch) -> PathBuf {
        self.cache
            .clone()
            .unwrap_or_else(|| launch.temp.path().join("caches"))
    }

    /// Whether a command can still start in it: on Linux the network of a thread whose
    /// commands are kept to the outbound proxy can end, and the next is prepared anew.
    pub(crate) fn lasts(&self) -> bool {
        self.enforcer.lasts()
    }

    /// How to start the next command, from the workspace as it is now, in a temp folder of
    /// its own.
    pub(crate) async fn launch(&self) -> Result<Launch, ComputerError> {
        let temp = RunTemp::new(&self.temps.parent, self.temps.prefix)
            .await
            .map_err(|error| {
                ComputerError::Failed(format!("could not make the command's temp folder: {error}"))
            })?;
        self.enforcer.launch(temp).await
    }
}

/// The folder in glave's own where sandboxed commands keep package caches between runs.
pub(crate) const CACHE_FOLDER: &str = "sandbox-cache";

/// glave's sandbox cache folder, made private if it is not there yet, resolved; none without
/// glave's own folder.
async fn cache_home(glave_home: Option<&Path>) -> Option<PathBuf> {
    let glave_home = tokio::fs::canonicalize(glave_home?).await.ok()?;
    let cache = glave_home.join(CACHE_FOLDER);
    let mut builder = tokio::fs::DirBuilder::new();
    #[cfg(unix)]
    builder.mode(0o700);
    match builder.create(&cache).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return None,
    }
    // A link planted in its place would lead the cache, and its writes, elsewhere.
    let meta = tokio::fs::symlink_metadata(&cache).await.ok()?;
    meta.is_dir().then_some(cache)
}

/// A path resolved if it exists; one that does not exist needs no protecting yet.
#[cfg(any(target_os = "macos", target_os = "linux"))]
async fn resolved(path: Option<&Path>) -> Option<PathBuf> {
    match path {
        Some(path) => tokio::fs::canonicalize(path).await.ok(),
        None => None,
    }
}

/// What a write in `root` could change that makes code run for everything the person does: git's
/// settings and every link on the way to them, followed even where the end isn't there yet, and
/// on request glave's own folder, when `root` holds it (one inside it is a thread's worktree, and
/// stays writable). A project inside what is kept is kept whole. Blocking: it follows links.
pub(crate) fn kept_read_only(
    root: &Path,
    home: Option<&Path>,
    glave_home: Option<&Path>,
) -> Vec<PathBuf> {
    let mut kept: Vec<PathBuf> = home
        .into_iter()
        .flat_map(git_settings)
        .flat_map(|named| on_the_way_to(&named))
        .filter_map(|place| {
            if place.starts_with(root) {
                Some(place)
            } else {
                root.starts_with(&place).then(|| root.to_owned())
            }
        })
        .collect();
    kept.extend(
        glave_home
            .filter(|home| home.starts_with(root))
            .map(Path::to_path_buf),
    );
    kept.sort();
    kept.dedup();
    kept
}

/// The most links followed on the way to one path, as an operating system limits them.
const MOST_LINKS: usize = 40;

/// Each link on the way to `path` and where it ends, followed one name at a time: removing a
/// link on the way, or making what it points to, changes what `path` is as surely as writing it.
fn on_the_way_to(path: &Path) -> Vec<PathBuf> {
    let mut places = Vec::new();
    let mut reached = PathBuf::new();
    let mut names: Vec<OsString> = path
        .components()
        .rev()
        .map(|name| name.as_os_str().to_owned())
        .collect();
    let mut links = 0;
    while let Some(name) = names.pop() {
        if name == ".." {
            reached.pop();
            continue;
        }
        let next = reached.join(&name);
        match std::fs::read_link(&next) {
            Ok(target) if links < MOST_LINKS => {
                links += 1;
                places.push(next);
                if target.is_absolute() {
                    reached = PathBuf::new();
                }
                names.extend(
                    target
                        .components()
                        .rev()
                        .map(|name| name.as_os_str().to_owned()),
                );
            }
            _ => reached = next,
        }
    }
    places.push(reached);
    places
}

/// The folders between `root` and each of `kept`, which would carry what is kept along if moved:
/// a rename is judged by its two paths, not by what the moved folder holds.
#[cfg(any(target_os = "macos", target_os = "linux", test))]
pub(crate) fn folders_leading_to<'a>(
    root: &'a Path,
    kept: &'a [PathBuf],
) -> impl Iterator<Item = PathBuf> + 'a {
    let mut folders: Vec<PathBuf> = kept
        .iter()
        .flat_map(|kept| kept.ancestors().skip(1))
        .filter(|folder| folder.starts_with(root) && *folder != root)
        .map(Path::to_path_buf)
        .collect();
    folders.sort();
    folders.dedup();
    folders.into_iter()
}

/// A worktree's own folder in its repository's shared git folder, from the `gitdir:` line
/// of its `.git` file, if it is where a worktree's folder belongs. Blocking.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn own_git_folder(root: &Path, common: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(root.join(".git")).ok()?;
    let line = text.lines().find_map(|line| line.strip_prefix("gitdir:"))?;
    let own = std::fs::canonicalize(root.join(line.trim())).ok()?;
    (own.parent() == Some(&common.join("worktrees"))).then_some(own)
}

/// What preparing the sandbox failed on.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn failed(error: &std::io::Error) -> ComputerError {
    ComputerError::Failed(format!("could not prepare the sandbox: {error}"))
}
