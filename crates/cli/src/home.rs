//! Where glave keeps its data on this computer.

use std::path::PathBuf;

use anyhow::{Context as _, bail};
use glave_process::Processes;

/// glave's folder: `$GLAVE_HOME`, or `~/.glave`.
pub(crate) fn folder() -> anyhow::Result<PathBuf> {
    match std::env::var_os("GLAVE_HOME") {
        Some(home) => Ok(PathBuf::from(home)),
        None => match std::env::var_os("HOME") {
            Some(home) => Ok(PathBuf::from(home).join(".glave")),
            None => bail!("cannot find your home folder; set GLAVE_HOME"),
        },
    }
}

/// Where this glave's processes are recorded while they run: its folder's `run`, once what a
/// glave that ended without stopping them left running is stopped, as glave starts.
pub(crate) async fn reaped() -> anyhow::Result<Processes> {
    let processes = Processes::recorded(folder()?.join("run"));
    match processes.reap().await {
        Ok(0) => {}
        Ok(stopped) => {
            eprintln!("glave: stopped {stopped} programs a glave that ended had left running");
        }
        Err(error) => eprintln!("glave: {error}"),
    }
    Ok(processes)
}

/// The person's home folder, where their own MCP servers start: `$HOME`, else glave's folder.
pub(crate) fn person() -> anyhow::Result<PathBuf> {
    match std::env::var_os("HOME") {
        Some(home) => Ok(PathBuf::from(home)),
        None => folder(),
    }
}

/// Where new projects' folders are made (`specs/storage.md`, "Projects"): `~/glave`, where the
/// person finds and opens them; beside a folder `GLAVE_HOME` names instead, as
/// `<that folder>-projects`, so a separate home keeps its own. Never inside glave's folder.
pub(crate) fn projects() -> anyhow::Result<PathBuf> {
    if let Some(home) = std::env::var_os("GLAVE_HOME") {
        let home = PathBuf::from(home);
        let Some(name) = home.file_name() else {
            bail!("GLAVE_HOME ({}) names no folder", home.display());
        };
        let mut beside = name.to_os_string();
        beside.push("-projects");
        return Ok(home.with_file_name(beside));
    }
    match std::env::var_os("HOME") {
        Some(home) => Ok(PathBuf::from(home).join("glave")),
        None => bail!("cannot find your home folder; set GLAVE_HOME"),
    }
}

/// Whether `GLAVE_HOME` moved glave's folder from `~/.glave`.
pub(crate) fn relocated() -> bool {
    std::env::var_os("GLAVE_HOME").is_some()
}

/// glave's folder, made if it is not there yet and readable only by you: content is sealed,
/// but the envelope of each event is not yet, and the local token is kept there too.
pub(crate) async fn private() -> anyhow::Result<PathBuf> {
    let home = folder()?;
    let mut builder = tokio::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    builder.mode(0o700);
    builder
        .create(&home)
        .await
        .with_context(|| format!("could not create {}", home.display()))?;
    // Creation only applies the mode to a new folder; an existing one is tightened too.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        tokio::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700))
            .await
            .with_context(|| format!("could not make {} private", home.display()))?;
    }
    Ok(home)
}

/// The database file, in glave's folder (made private first).
pub(crate) async fn database() -> anyhow::Result<PathBuf> {
    Ok(private().await?.join("glave.db"))
}
