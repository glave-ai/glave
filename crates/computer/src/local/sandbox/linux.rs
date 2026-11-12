//! The sandbox on Linux. bubblewrap first: a mount namespace whose root is read-only, with the
//! workspace writable but its protected paths laid over read-only, secret locations hidden,
//! a network namespace of the command's own when the network is off, its thread's when it
//! goes only through the outbound proxy, and a seccomp filter. Where bubblewrap cannot start,
//! Landlock and the same filter, at the `read-only` level only. Neither: a sandboxed command
//! is refused, saying what to install.

mod bubblewrap;
mod enter;
mod forward;
mod helper;
mod landlocked;
mod mounts;
mod names;
mod namespace;
mod seccomp;
mod temp;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use glave_ports::MAX_SOCKET_PATH;
use glave_ports::computer::{ComputerError, Sandbox, SandboxLevel, SandboxNetwork};
use glave_process::Processes;
use tokio::net::UnixListener;

use super::outbound::{Outbound, Serving, variables};
use super::run_temp::RunTemp;
use super::{Launch, Reached, SANDBOX_HELPER_ARG, Temps, failed, resolved};
use mounts::{Layout, StandIns};
use names::SecretNames;
use namespace::Namespace;
use temp::RunFolder;

pub(super) use enter::enter;
pub(super) use forward::forward;
pub(super) use helper::helper;

/// The protected paths' stand-ins a command holds until it ends (bubblewrap only).
pub(super) type Held = Option<mounts::Held>;

/// What to install, said whenever the sandbox cannot run.
const INSTALL: &str = "install bubblewrap (`sudo apt install bubblewrap`, \
    `sudo dnf install bubblewrap`); on Ubuntu 23.10 and later AppArmor must also let it \
    create user namespaces (a profile for `bwrap`, as `specs/sandbox.md` shows)";

pub(super) fn available() -> bool {
    bubblewrap::find(&[]).is_some_and(|bwrap| bubblewrap::starts(&bwrap).is_ok())
}

/// How commands are confined.
#[derive(Debug)]
enum How {
    Bubblewrap {
        bwrap: PathBuf,
        offline: bubblewrap::Offline,
        filter: Vec<u8>,
    },
    /// The helper program, which confines itself with Landlock.
    Landlock { helper: PathBuf },
}

/// Commands kept to the outbound proxy: the thread's network namespace, which glave's own
/// program joins for each command, and the proxy its forwarder reaches.
#[derive(Debug)]
struct Proxied {
    helper: PathBuf,
    namespace: Namespace,
    serving: Serving,
}

#[derive(Debug)]
pub(super) struct Enforcer {
    how: How,
    layout: Arc<Layout>,
    stand_ins: StandIns,
    /// Stopped before its socket's folder goes.
    proxied: Option<Proxied>,
    _folder: RunFolder,
}

impl Enforcer {
    pub(super) async fn prepare(
        sandbox: &Sandbox,
        root: &Path,
        git_common: Option<&Path>,
        helpers: (Option<&Path>, &Outbound, &Processes),
        reached: Reached<'_>,
        also_hidden: &[PathBuf],
    ) -> Result<(Self, Temps), ComputerError> {
        let git_common = resolved(git_common).await;
        let (helper, ..) = helpers;
        let how = choose(sandbox, root, git_common.as_deref(), helper).await?;
        let folder = tokio::task::spawn_blocking(RunFolder::new)
            .await
            .map_err(|error| ComputerError::Failed(error.to_string()))?
            .map_err(|error| failed(&error))?;
        let proxied = if sandbox.network == SandboxNetwork::Proxied {
            Some(proxied(&how, helpers, folder.run()).await?)
        } else {
            None
        };
        let names = SecretNames::new().map_err(|error| ComputerError::Failed(error.to_string()))?;
        let layout = Layout {
            root: root.to_owned(),
            level: sandbox.level,
            network: sandbox.network.is_on(),
            base: folder.base().to_owned(),
            empty_file: folder.empty_file(),
            empty_folder: folder.empty_folder(),
            shared_tmp: resolved(Some(Path::new("/tmp"))).await,
            home: resolved(sandbox.home.as_deref()).await,
            also_hidden: also_hidden.to_vec(),
            glave_home: resolved(sandbox.glave_home.as_deref()).await,
            cache: reached.cache.map(Path::to_path_buf),
            glave_on_path: resolved(reached.glave_on_path).await,
            runtime: resolved(Some(runtime_folder().as_path())).await,
            daemons: mounts::daemons(
                Some(&runtime_folder()),
                rustix::process::getuid().as_raw(),
                ssh_agent().as_deref(),
            ),
            git_common,
            names,
        };
        let temps = Temps {
            parent: folder.run().to_owned(),
            prefix: "tmp-",
            every: folder.base().to_owned(),
            every_prefix: "",
        };
        let enforcer = Self {
            how,
            layout: Arc::new(layout),
            stand_ins: Arc::new(Mutex::new(HashMap::new())),
            proxied,
            _folder: folder,
        };
        Ok((enforcer, temps))
    }

    /// Whether a command can still start in it: the network of a thread whose commands are kept
    /// to the proxy ends if its forwarder is stopped from outside glave.
    pub(super) fn lasts(&self) -> bool {
        self.proxied
            .as_ref()
            .is_none_or(|proxied| proxied.namespace.lasts())
    }

    pub(super) async fn launch(&self, temp: RunTemp) -> Result<Launch, ComputerError> {
        let layout = Arc::clone(&self.layout);
        let path = temp.path().to_owned();
        match &self.how {
            How::Bubblewrap {
                bwrap,
                offline,
                filter,
            } => {
                let stand_ins = Arc::clone(&self.stand_ins);
                let (mounts, held) =
                    tokio::task::spawn_blocking(move || mounts::plan(&layout, &path, &stand_ins))
                        .await
                        .map_err(|error| ComputerError::Failed(error.to_string()))?
                        .map_err(|refused| ComputerError::Refused(refused.into()))?;
                let args = bubblewrap::args(bwrap, *offline, &self.layout, &mounts);
                let (program, args, env) = match &self.proxied {
                    // Joined first, so bubblewrap starts the command in the thread's network.
                    Some(proxied) => {
                        let mut entering = proxied.namespace.entering();
                        entering.push(bubblewrap::SH.into());
                        entering.extend(args);
                        let env = variables(proxied.serving.secret(), proxied.namespace.port());
                        let env = env
                            .into_iter()
                            .map(|(name, value)| (name.into(), value.into()));
                        (proxied.helper.clone(), entering, env.collect())
                    }
                    None => (PathBuf::from(bubblewrap::SH), args, Vec::new()),
                };
                Ok(Launch {
                    program,
                    args,
                    env,
                    input: Some(filter.clone()),
                    own_session: false,
                    temp,
                    _held: Some(held),
                })
            }
            How::Landlock { helper } => {
                let plan = tokio::task::spawn_blocking(move || landlock_plan(&layout, &path))
                    .await
                    .map_err(|error| ComputerError::Failed(error.to_string()))?
                    .map_err(|refused| ComputerError::Refused(refused.into()))?;
                let plan = serde_json::to_string(&plan)
                    .map_err(|error| ComputerError::Failed(error.to_string()))?;
                Ok(Launch {
                    program: helper.clone(),
                    args: vec![SANDBOX_HELPER_ARG.into(), plan.into(), OsString::from("--")],
                    env: Vec::new(),
                    input: None,
                    own_session: true,
                    temp,
                    _held: None,
                })
            }
        }
    }
}

/// The user's runtime folder, where Podman keeps its sign-ins: `$XDG_RUNTIME_DIR`, or
/// `/run/user/<uid>` where that is unset.
fn runtime_folder() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|folder| folder.is_absolute())
        .unwrap_or_else(|| {
            let uid = rustix::process::getuid().as_raw();
            PathBuf::from(format!("/run/user/{uid}"))
        })
}

/// The ssh agent's socket, where `$SSH_AUTH_SOCK` names one.
fn ssh_agent() -> Option<PathBuf> {
    std::env::var_os("SSH_AUTH_SOCK")
        .map(PathBuf::from)
        .filter(|socket| socket.is_absolute())
}

/// bubblewrap if it starts here; otherwise Landlock if the level allows it and the kernel and
/// program have it; otherwise a refusal saying why and what to install.
async fn choose(
    sandbox: &Sandbox,
    root: &Path,
    git_common: Option<&Path>,
    helper: Option<&Path>,
) -> Result<How, ComputerError> {
    let writable: Vec<PathBuf> = [Some(root), git_common]
        .into_iter()
        .flatten()
        .map(Path::to_path_buf)
        .collect();
    // With the network, Unix sockets too: the mounts hide the person's daemons' (`mounts.rs`).
    let sockets = if sandbox.network.is_on() {
        seccomp::Sockets::InternetAndUnix
    } else {
        seccomp::Sockets::Offline
    };
    let tried = tokio::task::spawn_blocking(move || {
        let writable: Vec<&Path> = writable.iter().map(PathBuf::as_path).collect();
        let Some(bwrap) = bubblewrap::find(&writable) else {
            return Err("bubblewrap is not installed".to_owned());
        };
        let offline = bubblewrap::starts(&bwrap).map_err(|said| {
            format!("bubblewrap ({}) cannot start here: {said}", bwrap.display())
        })?;
        let filter = seccomp::program(sockets)?;
        Ok((bwrap, offline, seccomp::bytes(&filter)))
    })
    .await
    .map_err(|error| ComputerError::Failed(error.to_string()))?;
    let why = match tried {
        Ok((bwrap, offline, filter)) => {
            return Ok(How::Bubblewrap {
                bwrap,
                offline,
                filter,
            });
        }
        Err(why) => why,
    };
    let landlock = landlocked::available();
    if sandbox.level == SandboxLevel::ReadOnly
        && landlock
        && let Some(helper) = helper
    {
        return Ok(How::Landlock {
            helper: helper.to_owned(),
        });
    }
    let fallback = if landlock && sandbox.level == SandboxLevel::Project {
        " Landlock, the fallback, covers only the read-only level: it cannot keep git hooks \
         and settings read-only inside a writable project."
    } else {
        ""
    };
    Err(ComputerError::Unsupported(format!(
        "the sandbox cannot run on this computer: {why}. To use it, {INSTALL}.{fallback}"
    )))
}

/// The proxy for commands kept to it, started in `folder`, and their thread's network
/// namespace, held through `processes`, where `how` can make one and glave's own program,
/// `helper`, can forward in it and join it. Anywhere else they are refused rather than let out
/// directly: Landlock has no namespace, and under gVisor bubblewrap cannot make one.
async fn proxied(
    how: &How,
    (helper, outbound, processes): (Option<&Path>, &Outbound, &Processes),
    folder: &Path,
) -> Result<Proxied, ComputerError> {
    let why = match (how, helper) {
        (
            How::Bubblewrap {
                bwrap,
                offline: bubblewrap::Offline::OwnNetwork,
                ..
            },
            Some(helper),
        ) => {
            let (listener, socket) = proxy_socket(folder)?;
            let serving = Serving::start(outbound, listener)?;
            let namespace = Namespace::start(processes, bwrap, helper, &socket).await?;
            return Ok(Proxied {
                helper: helper.to_owned(),
                namespace,
                serving,
            });
        }
        (How::Bubblewrap { .. }, Some(_)) => {
            "bubblewrap cannot give commands a network namespace of their own here"
        }
        (How::Landlock { .. }, _) => "Landlock cannot give a command a network of its own",
        (_, None) => "glave's own program is not known, to forward in the thread's network",
    };
    Err(ComputerError::Unsupported(format!(
        "commands cannot be kept to glave's outbound proxy on this computer: {why}"
    )))
}

/// The socket's name in the sandbox's folder.
const SOCKET: &str = "proxy.sock";

/// A socket for the proxy in `folder`, private to this user, and its path.
fn proxy_socket(folder: &Path) -> Result<(UnixListener, PathBuf), ComputerError> {
    let socket = folder.join(SOCKET);
    if socket.as_os_str().len() > MAX_SOCKET_PATH {
        return Err(ComputerError::Failed(format!(
            "the outbound proxy's socket would be too long a path: {}",
            socket.display()
        )));
    }
    let listener = UnixListener::bind(&socket).map_err(|error| {
        ComputerError::Failed(format!(
            "could not start the outbound proxy at {}: {error}",
            socket.display()
        ))
    })?;
    Ok((listener, socket))
}

/// Every terminal's device in `dev` but `tty`, the command's own controlling terminal: `pts`,
/// where every pseudo-terminal is (the person's Terminal tab, a terminal window's, an ssh
/// session's), and the consoles and serial lines beside it (`tty1`, `ttyS0`), any of which
/// would let a command read what is typed there. bubblewrap's `/dev` is the sandbox's own,
/// holding none of them. Blocking.
fn other_terminals(dev: &Path) -> Vec<PathBuf> {
    let mut terminals = vec![dev.join("pts")];
    if let Ok(entries) = std::fs::read_dir(dev) {
        terminals.extend(
            entries
                .flatten()
                .map(|entry| entry.file_name())
                .filter(|name| {
                    let name = name.to_string_lossy();
                    name != "tty" && (name.starts_with("tty") || name.starts_with("pty"))
                })
                .map(|name| dev.join(name)),
        );
    }
    terminals
}

/// What the Landlock helper restricts itself to, from the workspace as it is now, for a
/// command whose temp folder is `temp`. Blocking.
fn landlock_plan(layout: &Layout, temp: &Path) -> Result<landlocked::Plan, String> {
    let mut unreadable: Vec<PathBuf> = mounts::hidden(layout)?;
    unreadable.extend(layout.glave_home.iter().cloned());
    unreadable.extend(other_terminals(Path::new("/dev")));
    let rereadable = layout
        .glave_home
        .iter()
        .filter(|glave_home| layout.root.starts_with(glave_home) && layout.root != **glave_home)
        .map(|_| layout.root.clone())
        .chain(layout.cache.iter().cloned())
        .chain(layout.glave_on_path.iter().cloned())
        .collect();
    unreadable.sort();
    unreadable.dedup();
    Ok(landlocked::Plan {
        network: layout.network,
        writable: layout
            .shared_tmp
            .iter()
            .map(PathBuf::as_path)
            .chain([temp])
            .chain(layout.cache.as_deref())
            .map(Path::to_path_buf)
            .collect(),
        // Not glave's sandbox folder: it guards bubblewrap's stand-ins, which a machine
        // without bubblewrap has none of, and excepting it would cost new files in `/tmp`.
        unwritable: vec![layout.root.clone()],
        rewritable: Vec::new(),
        unreadable,
        rereadable,
    })
}
