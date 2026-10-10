//! Looking at files where they are: what is at a path, which files match a pattern, and which
//! lines match an expression. Each is one call that runs next to the files and answers with
//! results, so nothing walks a filesystem across the wire.

use glave_event::Sha256;
use serde::{Deserialize, Serialize};

use super::types::EntryKind;

/// What is at a path (`files/stat`), symlinks followed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stat {
    /// The path resolved inside the workspace and made relative to its root, with `/`
    /// between names; `.` for the root. The same file always has the same path here.
    pub path: String,
    /// `None` when nothing is there yet.
    pub kind: Option<EntryKind>,
    /// In bytes; 0 for folders and for nothing.
    pub size: u64,
    /// The file's SHA-256, for files small enough to read whole; `None` otherwise. Lets the
    /// caller tell whether a file changed without fetching it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<Sha256>,
}

/// Which files to find (`files/glob`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Glob {
    /// A file pattern: without a `/` (`*.rs`) it matches file names in any folder; with one
    /// (`src/**/*.rs`) it matches paths from the folder searched.
    pub pattern: String,
    /// The folder to look in, relative to the workspace root.
    pub path: String,
    /// The most files returned.
    pub limit: usize,
}

/// Files found, newest first, leaving out what git ignores and the `.git` folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Globbed {
    /// The folder looked in, as a [`Stat`] path.
    pub path: String,
    /// Relative to the workspace root; at most the limit asked for.
    pub files: Vec<String>,
    /// How many matched in all.
    pub total: u64,
}

/// Which lines to find (`files/search`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Search {
    /// A regular expression; searched as plain text when it is not a valid one.
    pub pattern: String,
    /// The folder or file to search, relative to the workspace root.
    pub path: String,
    /// Only files matching this file pattern, read as [`Glob::pattern`] is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glob: Option<String>,
    #[serde(default)]
    pub ignore_case: bool,
    /// The most matching lines returned.
    pub limit: usize,
    /// Longest line returned; longer ones are cut and end in `…`.
    pub line_chars: usize,
}

/// One matching line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Match {
    /// Relative to the workspace root.
    pub path: String,
    /// Counted from 1.
    pub line: u64,
    pub text: String,
}

/// Lines found, in path order. Files git ignores, binary files and large files are not
/// searched, and files that usually hold secrets are never opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Searched {
    /// The folder or file searched, as a [`Stat`] path.
    pub path: String,
    pub matches: Vec<Match>,
    /// More lines matched than were returned.
    pub more: bool,
    /// Files skipped because they usually hold secrets, at most the limit asked for.
    pub secrets: Vec<String>,
    /// The pattern was not a valid regular expression, so it was searched as plain text.
    pub as_text: bool,
}

/// Whether bytes look like a binary file rather than text: a NUL byte, or more than 30%
/// control bytes, in the first 4 KiB.
pub fn looks_binary(bytes: &[u8]) -> bool {
    let head = bytes.get(..4_096).unwrap_or(bytes);
    if head.contains(&0) {
        return true;
    }
    let control = head
        .iter()
        .filter(|b| **b < 0x09 || (0x0E..0x20).contains(*b))
        .count();
    control.saturating_mul(10) > head.len().saturating_mul(3)
}
