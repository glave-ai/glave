//! Finding files and lines in a workspace the way git sees it: files by name or path, newest
//! first, and lines by regular expression, like ripgrep. Ignored, binary and large files are
//! not searched, and files that usually hold secrets are never opened.

use std::path::Path;
use std::time::SystemTime;

use glave_ports::secrets::is_secret_file;
use glave_ports::{ComputerError, Glob, Globbed, Match, Search, Searched, looks_binary};
use regex::{Regex, RegexBuilder};

use super::paths::{relative, resolve};
use super::walk::{self, Pattern};

/// Largest file searched; bigger files are usually data or build output.
const MAX_SEARCHED_BYTES: u64 = 2 * 1024 * 1024;

pub(crate) async fn glob(root: &Path, glob: &Glob) -> Result<Globbed, ComputerError> {
    let start = resolve(root, &glob.path).await?;
    let pattern = Pattern::new(&glob.pattern).map_err(ComputerError::Refused)?;
    let root = root.to_owned();
    let limit = glob.limit;
    tokio::task::spawn_blocking(move || {
        let mut files: Vec<(SystemTime, String)> = walk::files(&start)
            .filter(|path| pattern.matches(path.strip_prefix(&start).unwrap_or(path)))
            .map(|path| (modified(&path), relative(&root, &path)))
            .collect();
        // Newest first: what changed recently is what a task is usually about.
        files.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        let total = u64::try_from(files.len()).unwrap_or(u64::MAX);
        Globbed {
            path: relative(&root, &start),
            files: files
                .into_iter()
                .take(limit)
                .map(|(_, path)| path)
                .collect(),
            total,
        }
    })
    .await
    .map_err(|_| ComputerError::Failed("The file search stopped unexpectedly.".into()))
}

fn modified(path: &Path) -> SystemTime {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

pub(crate) async fn search(root: &Path, search: &Search) -> Result<Searched, ComputerError> {
    let start = resolve(root, &search.path).await?;
    let filter = search
        .glob
        .as_deref()
        .map(Pattern::new)
        .transpose()
        .map_err(ComputerError::Refused)?;
    let (regex, as_text) = compile(&search.pattern, search.ignore_case)?;
    let root = root.to_owned();
    let (limit, line_chars) = (search.limit, search.line_chars);
    tokio::task::spawn_blocking(move || {
        let mut found = Searched {
            path: relative(&root, &start),
            matches: Vec::new(),
            more: false,
            secrets: Vec::new(),
            as_text,
        };
        scan(
            &root,
            &start,
            &regex,
            filter.as_ref(),
            &mut found,
            (limit, line_chars),
        );
        found
    })
    .await
    .map_err(|_| ComputerError::Failed("The search stopped unexpectedly.".into()))
}

/// The pattern as a regular expression, or as plain text when it is not a valid one (`foo(`).
/// The flag says which.
fn compile(pattern: &str, ignore_case: bool) -> Result<(Regex, bool), ComputerError> {
    let build = |source: &str| {
        RegexBuilder::new(source)
            .case_insensitive(ignore_case)
            .build()
    };
    match build(pattern) {
        Ok(regex) => Ok((regex, false)),
        Err(_) => build(&regex::escape(pattern))
            .map(|regex| (regex, true))
            .map_err(|e| ComputerError::Refused(format!("The pattern could not be searched: {e}"))),
    }
}

fn scan(
    root: &Path,
    start: &Path,
    regex: &Regex,
    filter: Option<&Pattern>,
    found: &mut Searched,
    (limit, line_chars): (usize, usize),
) {
    for path in walk::files(start) {
        let relative_to_start = path.strip_prefix(start).unwrap_or(&path);
        if filter.is_some_and(|filter| !filter.matches(relative_to_start)) {
            continue;
        }
        if is_secret_file(&path) {
            if found.secrets.len() < limit {
                found.secrets.push(relative(root, &path));
            }
            continue;
        }
        let Some(text) = searchable(&path) else {
            continue;
        };
        for (index, line) in text.lines().enumerate() {
            if !regex.is_match(line) {
                continue;
            }
            if found.matches.len() >= limit {
                found.more = true;
                return;
            }
            found.matches.push(Match {
                path: relative(root, &path),
                line: u64::try_from(index.saturating_add(1)).unwrap_or(u64::MAX),
                text: clip(line, line_chars),
            });
        }
    }
}

/// The file's text, unless it is too big, unreadable or binary.
fn searchable(path: &Path) -> Option<String> {
    let size = std::fs::metadata(path).ok()?.len();
    if size > MAX_SEARCHED_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if looks_binary(&bytes) {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn clip(line: &str, most: usize) -> String {
    let line = line.trim_end_matches('\r');
    if line.chars().count() <= most {
        return line.to_owned();
    }
    let mut cut: String = line.chars().take(most).collect();
    cut.push('…');
    cut
}
