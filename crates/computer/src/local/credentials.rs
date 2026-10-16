//! Credentials for one command (`specs/computers.md`, rule 8). For the hosts the caller names,
//! git asks glave's credential helper, and each forge's own tool (`gh`, `glab`, `tea`) runs
//! through a small wrapper that asks the same helper and sets its token in that tool's own
//! environment only. The helper reaches glave over a connection on this machine's loopback
//! address, opened for this command and closed when it ends, and presents a random password
//! that only the command's helper file holds. Nothing secret is written: the files hold the
//! address and that password; the token travels only in answers, to the process that asked.

mod serving;

use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use glave_ports::{ComputerError, CredentialHost, Credentials, ForgeTool};
use tokio::net::TcpListener;

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
        // Holds nothing secret but the password, which dies with the listener.
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

impl Offered {
    /// Answers the command's helper until the command ends: run it beside the command.
    pub(crate) async fn serve(&self, credentials: &dyn Credentials) {
        serving::serve(&self.listener, &self.password, &self.hosts, credentials).await;
    }
}

/// The helper and wrappers for the hosts `credentials` names now, for a command whose
/// environment is `environment`; none when it names none.
pub(crate) async fn offer(
    credentials: &dyn Credentials,
    environment: &[(OsString, OsString)],
) -> Result<Option<Offered>, ComputerError> {
    let hosts: Vec<CredentialHost> = credentials
        .hosts()
        .await
        .into_iter()
        .filter(|host| serving::Place::of(&host.url).is_some())
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
    let helper = offered.folder.join("ask");
    write_script(&helper, &helper_script(port, &offered.password)).await?;
    let mut wrappers = false;
    for (tool, host) in ForgeTool::signed_in(&offered.hosts) {
        let script = wrapper(tool, host, &offered.folder, &helper);
        write_script(&offered.folder.join(tool.name()), &script).await?;
        wrappers = true;
    }
    offered.environment = settings(&offered.hosts, &helper, environment);
    if wrappers {
        let path = environment
            .iter()
            .find(|(name, _)| name == "PATH")
            .map(|(_, value)| value.clone());
        let mut first = offered.folder.clone().into_os_string();
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
/// answer. Anything else (`store`, `erase`) it ignores, so git keeps nothing anywhere.
fn helper_script(port: u16, password: &str) -> String {
    format!(
        "#!/usr/bin/env bash\n\
         # glave's credential helper for one command: it asks glave, at each use, for the\n\
         # credential of a host the person connected. Only `get` is answered.\n\
         test \"$1\" = get || exit 0\n\
         {{ exec 3<>/dev/tcp/127.0.0.1/{port}; }} 2>/dev/null || exit 0\n\
         {{ printf '%s\\n' '{password}'; cat; printf '\\n'; }} >&3\n\
         cat <&3\n"
    )
}

/// A forge tool's wrapper: asks the helper for `host`'s token now, sets it in the tool's own
/// environment only, and runs the real tool, found on `PATH` without the wrappers' folder.
fn wrapper(tool: ForgeTool, host: &CredentialHost, folder: &Path, helper: &Path) -> String {
    let name = tool.name();
    let (protocol, address) = host.url.split_once("://").unwrap_or(("https", &host.url));
    let address = address.split('/').next().unwrap_or(address);
    let mut script = format!(
        "#!/bin/sh\n\
         # {name}, signed in to {url} by glave for this one process: its token is asked for\n\
         # now and set only in {name}'s own environment.\n\
         wrappers={folder}\n\
         path=\nIFS=:\nset -f\n\
         for dir in $PATH; do [ \"$dir\" = \"$wrappers\" ] || path=\"${{path:+$path:}}$dir\"; done\n\
         set +f\nunset IFS\nPATH=$path\n\
         command -v {name} >/dev/null 2>&1 || {{ echo \"{name}: command not found\" >&2; exit 127; }}\n\
         token=$(printf 'protocol=%s\\nhost=%s\\n' {protocol} {address} | {helper} get | \
         sed -n 's/^password=//p')\n\
         if [ -n \"$token\" ]; then\n",
        url = host.url,
        folder = quoted(&folder.to_string_lossy()),
        protocol = quoted(protocol),
        address = quoted(address),
        helper = quoted(&helper.to_string_lossy()),
    );
    for (variable, value) in variables(tool, host) {
        let value = value.map_or_else(|| "$token".to_owned(), |value| quoted(&value));
        let _ = writeln!(script, "  {variable}={value}; export {variable}");
    }
    let _ = write!(script, "fi\nexec {name} \"$@\"\n");
    script
}

/// What each tool reads its token and host from (its documentation's names); `None` stands
/// for the token.
fn variables(tool: ForgeTool, host: &CredentialHost) -> Vec<(&'static str, Option<String>)> {
    let home = Some(host.url.as_str()) == tool.home();
    let bare = host
        .url
        .split_once("://")
        .map_or(host.url.as_str(), |(_, rest)| rest)
        .to_owned();
    match tool {
        ForgeTool::Gh if home => vec![("GH_TOKEN", None)],
        ForgeTool::Gh => vec![("GH_ENTERPRISE_TOKEN", None), ("GH_HOST", Some(bare))],
        ForgeTool::Glab if home => vec![("GITLAB_TOKEN", None)],
        ForgeTool::Glab => vec![
            ("GITLAB_TOKEN", None),
            ("GITLAB_HOST", Some(host.url.clone())),
        ],
        ForgeTool::Tea => vec![
            ("GITEA_TOKEN", None),
            ("GITEA_INSTANCE_URL", Some(host.url.clone())),
        ],
    }
}

/// `text` in single quotes, for a shell.
fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
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
