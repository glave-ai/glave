//! The sandbox on macOS: `sandbox-exec` with a Seatbelt profile written once per workspace,
//! and each command's own temp folder given with it.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use glave_ports::computer::{ComputerError, Sandbox};

use super::profile::{Places, TEMP_PARAM, home_secrets, profile};
use super::run_temp::RunTemp;
use super::{Launch, Temps, failed, own_git_folder, resolved};

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

/// The profile's arguments, but for each command's own temp folder.
#[derive(Debug)]
pub(super) struct Enforcer {
    args: Vec<OsString>,
}

impl Enforcer {
    /// Resolves every path the profile names (macOS matches real paths) and writes the
    /// profile once, for every command run in `root` under `sandbox`. Commands' temp folders
    /// go in the system's, resolved.
    pub(super) async fn prepare(
        sandbox: &Sandbox,
        root: &Path,
        git_common: Option<&Path>,
        _helper: Option<&Path>,
        cache: Option<&Path>,
    ) -> Result<(Self, Temps), ComputerError> {
        if !available() {
            return Err(ComputerError::Unsupported(format!(
                "the sandbox is not available on this computer: {SANDBOX_EXEC} is missing"
            )));
        }
        let system_temp = tokio::fs::canonicalize(std::env::temp_dir())
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
            glave_home: resolved(sandbox.glave_home.as_deref()).await,
            hidden,
            cache: cache.map(Path::to_path_buf),
            git_common,
            git_own,
        };
        let profile = profile(&places, sandbox.level, sandbox.network);
        let mut args: Vec<OsString> = vec!["-p".into(), profile.text.into()];
        args.extend(profile.params.iter().map(|(name, path)| param(name, path)));
        let temps = Temps {
            parent: system_temp,
            prefix: "glave-sandbox-",
        };
        Ok((Self { args }, temps))
    }

    /// A future, as on Linux, which surveys the workspace before each command.
    pub(super) fn launch(
        &self,
        temp: RunTemp,
    ) -> std::future::Ready<Result<Launch, ComputerError>> {
        let mut args = self.args.clone();
        args.push(param(TEMP_PARAM, temp.path()));
        args.push("--".into());
        std::future::ready(Ok(Launch {
            program: PathBuf::from(SANDBOX_EXEC),
            args,
            input: None,
            own_session: false,
            temp,
            _held: (),
        }))
    }
}

/// `-D<name>=<path>`: one of a profile's parameters.
fn param(name: &str, path: &Path) -> OsString {
    let mut arg = OsString::from(format!("-D{name}="));
    arg.push(path);
    arg
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
