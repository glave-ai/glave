//! The sandbox on macOS: `sandbox-exec` with a Seatbelt profile written once per workspace.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use glave_ports::{ComputerError, Sandbox};

use super::profile::{Places, home_secrets, profile};
use super::{Launch, failed, own_git_folder, resolved};

/// The only `sandbox-exec` trusted: an earlier one in `PATH` could be planted.
const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// Nothing is left behind per command on macOS.
pub(super) type Held = ();

pub(super) fn available() -> bool {
    Path::new(SANDBOX_EXEC).is_file()
}

pub(super) fn helper(_args: &[OsString]) -> String {
    "the sandbox helper runs only on Linux".into()
}

/// The profile's arguments, and the temp folder it grants.
#[derive(Debug)]
pub(super) struct Enforcer {
    args: Vec<OsString>,
    temp: PathBuf,
}

impl Enforcer {
    /// Resolves every path the profile names (macOS matches real paths) and writes the
    /// profile once, for every command run in `root` under `sandbox`.
    pub(super) async fn prepare(
        sandbox: &Sandbox,
        root: &Path,
        git_common: Option<&Path>,
        _helper: Option<&Path>,
        cache: Option<&Path>,
    ) -> Result<(Self, PathBuf), ComputerError> {
        if !available() {
            return Err(ComputerError::Unsupported(format!(
                "the sandbox is not available on this computer: {SANDBOX_EXEC} is missing"
            )));
        }
        let system_temp = tokio::fs::canonicalize(std::env::temp_dir())
            .await
            .map_err(|error| failed(&error))?;
        let temp = run_temp(&system_temp)
            .await
            .map_err(|error| failed(&error))?;
        let git_common = resolved(git_common).await;
        let git_own = match git_common.clone() {
            Some(common) => {
                let root = root.to_owned();
                tokio::task::spawn_blocking(move || own_git_folder(&root, &common))
                    .await
                    .ok()
                    .flatten()
            }
            None => None,
        };
        let hidden = match resolved(sandbox.home.as_deref()).await {
            Some(home) => hidden(&home).await,
            None => Vec::new(),
        };
        let places = Places {
            root: root.to_owned(),
            temp: temp.clone(),
            glave_home: resolved(sandbox.glave_home.as_deref()).await,
            hidden,
            cache: cache.map(Path::to_path_buf),
            git_common,
            git_own,
        };
        let profile = profile(&places, sandbox.level, sandbox.network);
        let mut args: Vec<OsString> = vec!["-p".into(), profile.text.into()];
        for (name, path) in profile.params {
            let mut arg = OsString::from(format!("-D{name}="));
            arg.push(path);
            args.push(arg);
        }
        args.push("--".into());
        Ok((
            Self {
                args,
                temp: temp.clone(),
            },
            temp,
        ))
    }

    /// A future, as on Linux, which surveys the workspace before each command.
    pub(super) fn launch(&self) -> std::future::Ready<Result<Launch, ComputerError>> {
        std::future::ready(Ok(Launch {
            program: PathBuf::from(SANDBOX_EXEC),
            args: self.args.clone(),
            input: None,
            own_session: false,
            _held: (),
        }))
    }
}

impl Drop for Enforcer {
    fn drop(&mut self) {
        // Best effort: the folder is in the system's temp folder, which is cleaned anyway.
        let _ = std::fs::remove_dir_all(&self.temp);
    }
}

/// The home folder's credential stores, each as named and, when a link leads elsewhere, where
/// it leads: Seatbelt matches the real path, so a `~/.config` that is a link to dotfiles would
/// otherwise leave what it holds readable.
async fn hidden(home: &Path) -> Vec<PathBuf> {
    let mut hidden = Vec::new();
    for path in home_secrets(home) {
        if let Ok(real) = tokio::fs::canonicalize(&path).await
            && real != path
        {
            hidden.push(real);
        }
        hidden.push(path);
    }
    hidden
}

/// A temp folder of the sandbox's own inside the system's, so commands cannot write other
/// programs' temp files.
async fn run_temp(system_temp: &Path) -> std::io::Result<PathBuf> {
    let pid = std::process::id();
    for attempt in 0_u32..1_000 {
        let dir = system_temp.join(format!("glave-sandbox-{pid}-{attempt}"));
        match tokio::fs::create_dir(&dir).await {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not make a temp folder for the sandbox",
    ))
}
