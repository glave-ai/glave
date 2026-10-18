//! Credentials for one command (`specs/computers.md`, rule 8). For the hosts the caller names,
//! git asks glave's credential helper, and a forge tool that keeps its token to one host
//! (`gh` for github.com, `tea`) runs through a small wrapper that asks the same helper and sets
//! the token in that tool's own environment only. The helper reaches glave over a connection
//! on this machine's loopback address, opened for this command and closed when it ends, and
//! presents a random password that only the helper's file holds. Nothing secret is written:
//! the files hold the address and that password; the token travels only in answers, to the
//! process that asked.

mod serving;
mod wrapper;

use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use glave_ports::computer::{ComputerError, CredentialHost, ForgeTool, GitCredentials};
use tokio::net::TcpListener;
use wrapper::{Places, quoted};

/// The helper's name: off `PATH`, and named as git's helpers are, so a command that runs it
/// itself is read as printing a credential (`glave-shell`).
const HELPER: &str = "git-credential-glave";

/// The helper git runs for each connected host, and the forge tools' wrappers, in a folder of
/// their own for one command, removed when it ends.
pub(crate) struct Offered {
    folder: PathBuf,
    listener: TcpListener,
    password: String,
    hosts: Vec<CredentialHost>,
    /// What the command's environment gains: git's settings naming the helper, and the
    /// wrappers' folder first in `PATH`.
    pub(crate) environment: Vec<(OsString, OsString)>,
}

impl Drop for Offered {
    fn drop(&mut self) {
        // Holds nothing secret but the password, which dies with the listener. Small, so
        // removing it does not hold the runtime up. The tools' read-only folders, or any a
        // command made unwritable, are made writable first, so nothing is left behind.
        #[cfg(unix)]
        if let Ok(entries) = std::fs::read_dir(&self.folder) {
            use std::os::unix::fs::PermissionsExt as _;
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    let writable = std::fs::Permissions::from_mode(0o700);
                    let _ = std::fs::set_permissions(entry.path(), writable);
                }
            }
        }
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

impl Offered {
    /// Answers the command's helper until the command ends: run it beside the command.
    pub(crate) async fn serve(&self, credentials: &dyn GitCredentials) {
        serving::serve(&self.listener, &self.password, &self.hosts, credentials).await;
    }
}

/// The helper and wrappers for the hosts `credentials` names now, for a command whose
/// environment is `environment`; none when it names none. A host whose URL is not a plain
/// origin is never offered.
pub(crate) async fn offer(
    credentials: &dyn GitCredentials,
    environment: &[(OsString, OsString)],
) -> Result<Option<Offered>, ComputerError> {
    let hosts: Vec<CredentialHost> = credentials
        .hosts()
        .await
        .into_iter()
        .filter(|host| host.origin().is_some())
        .collect();
    if hosts.is_empty() {
        return Ok(None);
    }
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| failed("listen for the credential helper", &error))?;
    let port = listener
        .local_addr()
        .map_err(|error| failed("listen for the credential helper", &error))?
        .port();
    let password = random_hex()?;
    let folder = private_folder().await?;
    let mut offered = Offered {
        folder,
        listener,
        password,
        hosts,
        environment: Vec::new(),
    };
    let helper = offered.folder.join(HELPER);
    write_script(&helper, &helper_script(port, &offered.password)).await?;
    offered.environment = settings(&offered.hosts, &helper, environment);
    let tools = ForgeTool::signed_in(&offered.hosts);
    if !tools.is_empty() {
        let bin = offered.folder.join("bin");
        tokio::fs::create_dir(&bin)
            .await
            .map_err(|error| failed("make the wrappers' folder", &error))?;
        for (tool, host) in tools {
            let empty = offered.folder.join(format!("{}-home", tool.name()));
            read_only_folder(&empty).await?;
            let places = Places {
                bin: &bin,
                helper: &helper,
                empty: &empty,
            };
            let script = wrapper::wrapper(tool, host, &places, environment);
            write_script(&bin.join(tool.name()), &script).await?;
        }
        let path = environment
            .iter()
            .find(|(name, _)| name == "PATH")
            .map(|(_, value)| value.clone());
        let mut first = bin.into_os_string();
        if let Some(path) = path {
            first.push(":");
            first.push(path);
        }
        offered.environment.push(("PATH".into(), first));
    }
    Ok(Some(offered))
}

/// git's settings for this command, through `GIT_CONFIG_*` after any it has already: for each
/// host, the helpers configured elsewhere cleared (the empty value), then glave's.
fn settings(
    hosts: &[CredentialHost],
    helper: &Path,
    environment: &[(OsString, OsString)],
) -> Vec<(OsString, OsString)> {
    let before = environment
        .iter()
        .find(|(name, _)| name == "GIT_CONFIG_COUNT")
        .and_then(|(_, count)| count.to_str()?.parse::<usize>().ok())
        .unwrap_or(0);
    let run = format!("!{}", quoted(&helper.to_string_lossy()));
    let mut settings = Vec::new();
    let mut n = before;
    for host in hosts {
        let key = format!("credential.{}.helper", host.url);
        for value in [String::new(), run.clone()] {
            settings.push((format!("GIT_CONFIG_KEY_{n}").into(), key.clone().into()));
            settings.push((format!("GIT_CONFIG_VALUE_{n}").into(), value.into()));
            n = n.saturating_add(1);
        }
    }
    settings.push(("GIT_CONFIG_COUNT".into(), n.to_string().into()));
    settings
}

/// The helper: on `get`, it sends the password and git's request to glave and prints the
/// answer, waiting at most 30 seconds for each line. Anything else (`store`, `erase`) it
/// ignores, so this helper keeps nothing anywhere.
fn helper_script(port: u16, password: &str) -> String {
    format!(
        "#!/usr/bin/env bash\n\
         # glave's credential helper for one command: it asks glave, at each use, for the\n\
         # credential of a host the person connected. Only `get` is answered.\n\
         test \"$1\" = get || exit 0\n\
         {{ exec 3<>/dev/tcp/127.0.0.1/{port}; }} 2>/dev/null || exit 0\n\
         {{ printf '%s\\n' '{password}'; cat; printf '\\n'; }} >&3\n\
         while IFS= read -r -t 30 line <&3; do printf '%s\\n' \"$line\"; done\n"
    )
}

/// A folder only this user may enter, in the system's temp folder.
async fn private_folder() -> Result<PathBuf, ComputerError> {
    let folder = std::env::temp_dir().join(format!("glave-credentials-{}", random_hex()?));
    let mut builder = tokio::fs::DirBuilder::new();
    #[cfg(unix)]
    builder.mode(0o700);
    builder
        .create(&folder)
        .await
        .map_err(|error| failed("make the credential helper's folder", &error))?;
    Ok(folder)
}

/// An empty folder a tool may read and never write, for its configuration and extensions.
async fn read_only_folder(path: &Path) -> Result<(), ComputerError> {
    let mut builder = tokio::fs::DirBuilder::new();
    #[cfg(unix)]
    builder.mode(0o500);
    builder
        .create(path)
        .await
        .map_err(|error| failed("make a forge tool's folder", &error))
}

/// Writes a script only this user may read or run.
async fn write_script(path: &Path, text: &str) -> Result<(), ComputerError> {
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o700);
    let mut file = options
        .open(path)
        .await
        .map_err(|error| failed("write the credential helper", &error))?;
    tokio::io::AsyncWriteExt::write_all(&mut file, text.as_bytes())
        .await
        .map_err(|error| failed("write the credential helper", &error))
}

/// 32 random bytes as hex.
fn random_hex() -> Result<String, ComputerError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| {
        ComputerError::Failed(format!(
            "could not get random bytes for credentials: {error}"
        ))
    })?;
    Ok(bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    }))
}

fn failed(what: &str, error: &std::io::Error) -> ComputerError {
    ComputerError::Failed(format!("could not {what}: {error}"))
}

#[cfg(test)]
mod tests;
