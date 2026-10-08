//! Where glave keeps its data on this computer.

use std::path::PathBuf;

use anyhow::{Context as _, bail};

/// glave's folder: `$GLAVE_HOME`, or `~/.glave`.
pub fn folder() -> anyhow::Result<PathBuf> {
    match std::env::var_os("GLAVE_HOME") {
        Some(home) => Ok(PathBuf::from(home)),
        None => match std::env::var_os("HOME") {
            Some(home) => Ok(PathBuf::from(home).join(".glave")),
            None => bail!("cannot find your home folder; set GLAVE_HOME"),
        },
    }
}

/// The database file, in glave's folder. The folder is created readable only by you, since
/// the database is not yet encrypted.
pub async fn database() -> anyhow::Result<PathBuf> {
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
    Ok(home.join("glave.db"))
}
