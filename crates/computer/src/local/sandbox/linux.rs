//! The sandbox on Linux. bubblewrap first: a mount namespace whose root is read-only, with the
//! workspace writable but its protected paths laid over read-only, secret locations hidden,
//! the network namespace unshared when the network is off, and a seccomp filter. Where
//! bubblewrap cannot start, Landlock and the same filter, at the `read-only` level only.
//! Neither: a sandboxed command is refused, saying what to install.

mod bubblewrap;
mod helper;
mod landlocked;
mod mounts;
mod names;
mod seccomp;
mod temp;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use glave_ports::computer::{ComputerError, Sandbox, SandboxLevel};

use super::run_temp::RunTemp;
use super::{Launch, Reached, SANDBOX_HELPER_ARG, Temps, failed, resolved};
use mounts::{Layout, StandIns};
use names::SecretNames;
use temp::RunFolder;

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
        filter: Vec<u8>,
    },
    /// The helper program, which confines itself with Landlock.
    Landlock {
        helper: PathBuf,
    },
}

#[derive(Debug)]
pub(super) struct Enforcer {
    how: How,
    layout: Arc<Layout>,
    stand_ins: StandIns,
    _folder: RunFolder,
}

impl Enforcer {
    pub(super) async fn prepare(
        sandbox: &Sandbox,
        root: &Path,
        git_common: Option<&Path>,
        helper: Option<&Path>,
        reached: Reached<'_>,
        also_hidden: &[PathBuf],
    ) -> Result<(Self, Temps), ComputerError> {
        let git_common = resolved(git_common).await;
        let how = choose(sandbox, root, git_common.as_deref(), helper).await?;
        let folder = tokio::task::spawn_blocking(RunFolder::new)
            .await
            .map_err(|error| ComputerError::Failed(error.to_string()))?
            .map_err(|error| failed(&error))?;
        let names = SecretNames::new().map_err(|error| ComputerError::Failed(error.to_string()))?;
        let layout = Layout {
            root: root.to_owned(),
            level: sandbox.level,
            network: sandbox.network,
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
            git_common,
            names,
        };
        let temps = Temps {
            parent: folder.run().to_owned(),
            prefix: "tmp-",
        };
        let enforcer = Self {
            how,
            layout: Arc::new(layout),
            stand_ins: Arc::new(Mutex::new(HashMap::new())),
            _folder: folder,
        };
        Ok((enforcer, temps))
    }

    pub(super) async fn launch(&self, temp: RunTemp) -> Result<Launch, ComputerError> {
        let layout = Arc::clone(&self.layout);
        let path = temp.path().to_owned();
        match &self.how {
            How::Bubblewrap { bwrap, filter } => {
                let stand_ins = Arc::clone(&self.stand_ins);
                let (mounts, held) =
                    tokio::task::spawn_blocking(move || mounts::plan(&layout, &path, &stand_ins))
                        .await
                        .map_err(|error| ComputerError::Failed(error.to_string()))?
                        .map_err(ComputerError::Refused)?;
                Ok(Launch {
                    program: PathBuf::from(bubblewrap::SH),
                    args: bubblewrap::args(bwrap, &self.layout, &mounts),
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
                    .map_err(ComputerError::Refused)?;
                let plan = serde_json::to_string(&plan)
                    .map_err(|error| ComputerError::Failed(error.to_string()))?;
                Ok(Launch {
                    program: helper.clone(),
                    args: vec![SANDBOX_HELPER_ARG.into(), plan.into(), OsString::from("--")],
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
    let network = sandbox.network;
    let tried = tokio::task::spawn_blocking(move || {
        let writable: Vec<&Path> = writable.iter().map(PathBuf::as_path).collect();
        let Some(bwrap) = bubblewrap::find(&writable) else {
            return Err("bubblewrap is not installed".to_owned());
        };
        bubblewrap::starts(&bwrap).map_err(|said| {
            format!("bubblewrap ({}) cannot start here: {said}", bwrap.display())
        })?;
        let filter = seccomp::program(network)?;
        Ok((bwrap, seccomp::bytes(&filter)))
    })
    .await
    .map_err(|error| ComputerError::Failed(error.to_string()))?;
    let why = match tried {
        Ok((bwrap, filter)) => return Ok(How::Bubblewrap { bwrap, filter }),
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

/// What the Landlock helper restricts itself to, from the workspace as it is now, for a
/// command whose temp folder is `temp`. Blocking.
fn landlock_plan(layout: &Layout, temp: &Path) -> Result<landlocked::Plan, String> {
    let mut unreadable: Vec<PathBuf> = mounts::hidden(layout)?;
    unreadable.extend(layout.glave_home.iter().cloned());
    let rereadable = layout
        .glave_home
        .iter()
        .filter(|glave_home| layout.root.starts_with(glave_home))
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
