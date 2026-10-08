//! Locating the workspace root.

use std::path::{Path, PathBuf};

/// The workspace root: the parent of the `xtask` crate.
pub fn root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "could not locate the workspace root".to_owned())
}
