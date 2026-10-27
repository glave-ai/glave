//! glave's own folder on commands' `PATH` (see [`crate::LocalComputers::with_glave_on_path`]):
//! it holds `glave`, a link to the glave that runs them, so a command reaches that glave
//! whatever else is installed, and none at all. Made at each start, as Codex links its own
//! binary for its `apply_patch` (`codex-rs/arg0`): a link to the binary's path, so an upgrade
//! that replaces the binary in place keeps it working.

use std::io;
use std::path::Path;

/// Makes `folder` and in it `glave`, a link to `program`, replacing any link there: a new one
/// at another name is put in its place in one step, so a command never finds none.
#[cfg(unix)]
pub async fn glave_on_path(folder: &Path, program: &Path) -> io::Result<()> {
    let mut builder = tokio::fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder.create(folder).await?;
    let writing = folder.join(format!(".glave.{}", std::process::id()));
    match tokio::fs::remove_file(&writing).await {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    tokio::fs::symlink(program, &writing).await?;
    tokio::fs::rename(&writing, folder.join("glave")).await
}

/// Where links are not made this way: glave's folder is not put on commands' `PATH`.
#[cfg(not(unix))]
pub async fn glave_on_path(_folder: &Path, _program: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "glave is put on commands' PATH on macOS and Linux only",
    ))
}
