//! The sandbox on macOS: `sandbox-exec` with a Seatbelt profile written once per workspace,
//! and each command's own temp folder given with it. Commands kept to the outbound proxy reach
//! only this computer's loopback, where their sandbox's proxy listens.

use std::ffi::OsString;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

use glave_ports::computer::{ComputerError, Sandbox, SandboxNetwork};

use super::outbound::{Serving, variables};
use super::profile::{Places, TEMP_PARAM, home_secrets, profile};
use super::run_temp::RunTemp;
use super::{Launch, Outbound, Reached, Temps, failed, kept_read_only, own_git_folder, resolved};

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

pub(super) fn forward(_args: &[OsString]) -> Result<std::process::ExitCode, String> {
    Err("the forwarder runs only on Linux".into())
}

/// The profile's arguments, but for each command's own temp folder; and for commands kept to
/// the outbound proxy, the proxy, which stops when this is dropped.
#[derive(Debug)]
pub(super) struct Enforcer {
    args: Vec<OsString>,
    proxied: Option<Proxied>,
}

/// The proxy commands are kept to, and the port of loopback it listens on.
#[derive(Debug)]
struct Proxied {
    serving: Serving,
    port: u16,
}

impl Enforcer {
    /// Resolves every path the profile names (macOS matches real paths) and writes the
    /// profile once, for every command run in `root` under `sandbox`, hiding the home
    /// folder's credential stores and `also_hidden`. Commands' temp folders go in the
    /// system's, resolved.
    pub(super) async fn prepare(
        sandbox: &Sandbox,
        root: &Path,
        git_common: Option<&Path>,
        (_helper, outbound): (Option<&Path>, &Outbound),
        reached: Reached<'_>,
        also_hidden: &[PathBuf],
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
        let home = resolved(sandbox.home.as_deref()).await;
        let secrets: Vec<PathBuf> = match &home {
            Some(home) => home_secrets(home).collect(),
            None => Vec::new(),
        };
        let hidden = hidden(secrets.into_iter().chain(also_hidden.iter().cloned())).await;
        let glave_home = resolved(sandbox.glave_home.as_deref()).await;
        let kept = {
            let (root, glave_home) = (root.to_owned(), glave_home.clone());
            tokio::task::spawn_blocking(move || {
                kept_read_only(&root, home.as_deref(), glave_home.as_deref())
            })
            .await
            .map_err(|error| ComputerError::Failed(error.to_string()))?
        };
        let places = Places {
            root: root.to_owned(),
            glave_home,
            kept,
            hidden,
            cache: reached.cache.map(Path::to_path_buf),
            glave_on_path: resolved(reached.glave_on_path).await,
            git_common,
            git_own,
        };
        let proxied = match sandbox.network {
            SandboxNetwork::Proxied => Some(proxied(outbound)?),
            SandboxNetwork::Off | SandboxNetwork::Direct => None,
        };
        let profile = profile(&places, sandbox.level, sandbox.network);
        let mut args: Vec<OsString> = vec!["-p".into(), profile.text.into()];
        args.extend(profile.params.iter().map(|(name, path)| param(name, path)));
        let temps = Temps {
            every: system_temp.clone(),
            every_prefix: "glave-sandbox-",
            parent: system_temp,
            prefix: "glave-sandbox-",
        };
        Ok((Self { args, proxied }, temps))
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
            env: self
                .proxied
                .iter()
                .flat_map(|proxied| variables(proxied.serving.secret(), proxied.port))
                .map(|(name, value)| (name.into(), value.into()))
                .collect(),
            input: None,
            own_session: false,
            temp,
            _held: (),
        }))
    }
}

/// `outbound`'s proxy, started on a port of this computer's loopback: every sandboxed command
/// reaches every port there, so only its secret keeps another thread's commands out.
fn proxied(outbound: &Outbound) -> Result<Proxied, ComputerError> {
    let failed = |error: std::io::Error| {
        ComputerError::Failed(format!(
            "could not start the outbound proxy on this computer's loopback: {error}"
        ))
    };
    let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(failed)?;
    let port = listener.local_addr().map_err(failed)?.port();
    listener.set_nonblocking(true).map_err(failed)?;
    let listener = tokio::net::TcpListener::from_std(listener).map_err(failed)?;
    Ok(Proxied {
        serving: Serving::start(outbound, listener)?,
        port,
    })
}

/// `-D<name>=<path>`: one of a profile's parameters.
fn param(name: &str, path: &Path) -> OsString {
    let mut arg = OsString::from(format!("-D{name}="));
    arg.push(path);
    arg
}

/// Each of `paths`, as named and, when a link leads elsewhere, where it leads: Seatbelt matches
/// the real path, so a `~/.config` that is a link to dotfiles would otherwise leave what it
/// holds readable.
async fn hidden(paths: impl Iterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut hidden = Vec::new();
    for path in paths {
        if let Ok(real) = tokio::fs::canonicalize(&path).await
            && real != path
        {
            hidden.push(real);
        }
        hidden.push(path);
    }
    hidden
}
