//! Reports saved as text files, one line per entry.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Writes `lines` to `report.txt` in `dir`, each ending in a newline, and returns its path.
pub fn save(dir: &Path, lines: &[&str]) -> io::Result<PathBuf> {
    let path = dir.join("report.txt");
    fs::write(&path, lines.join("\n"))?;
    Ok(path)
}

/// The lines of the report at `path`.
pub fn load(path: &Path) -> io::Result<Vec<String>> {
    Ok(fs::read_to_string(path)?
        .split_terminator('\n')
        .map(str::to_owned)
        .collect())
}
