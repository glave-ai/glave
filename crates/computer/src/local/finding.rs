//! Finding files and lines in a workspace the way git sees it: files by name or path, newest
//! first, and lines by regular expression, like ripgrep. Ignored, binary and large files are
//! not searched, and files that usually hold secrets are never opened. The walk goes by path,
//! so every file it finds is looked up again from the workspace root with no link along the
//! way (`plain`) and read through that lookup: a folder swapped for a link mid-walk shows
//! neither names nor text from outside.

use std::io::Read as _;
use std::os::fd::AsFd as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use glave_ports::computer::{ComputerError, Glob, Globbed, Match, Search, Searched, looks_binary};
use glave_ports::secrets::is_secret_file;
use glave_ports::{Clock as _, SystemClock};
use regex::{Regex, RegexBuilder};
use rustix::fs::Stat;

use super::anchored::{Root, count, open_to_read, plain, resolve, shown};
use super::files::blocking;
use super::paths::relative;
use super::walk::{self, Pattern};

/// Largest file searched; bigger files are usually data or build output.
const MAX_SEARCHED_BYTES: u64 = 2 * 1024 * 1024;

/// The longest a look with a limit on what it looks at looks ([`Glob::most_looked_at`]).
const LOOKING_FOR: Duration = Duration::from_secs(2);

pub(crate) async fn glob(root: &Path, glob: &Glob) -> Result<Globbed, ComputerError> {
    let pattern =
        Pattern::new(&glob.pattern).map_err(|refused| ComputerError::Refused(refused.into()))?;
    let (root, glob) = (root.to_owned(), glob.clone());
    blocking(move || {
        let root = Root::open(&root)?;
        let found = resolve(&root, &glob.path)?;
        let top = root.path.join(found.names.iter().collect::<PathBuf>());
        let since = glob
            .changed_since_ns
            .map(|nanos| SystemTime::UNIX_EPOCH + Duration::from_nanos(nanos));
        let now = SystemTime::from(SystemClock.now());
        // A look over a window of time ends at now, or as long before it as files must have
        // settled: the next look starts just after.
        let windowed = glob.changed_since_ns.is_some() || glob.settled_for_ns.is_some();
        let cut_off = windowed.then(|| {
            let settled = Duration::from_nanos(glob.settled_for_ns.unwrap_or(0));
            now.checked_sub(settled).unwrap_or(SystemTime::UNIX_EPOCH)
        });
        let started = Instant::now();
        let mut stopped = false;
        let mut files: Vec<(SystemTime, String)> = Vec::new();
        for (looked_at, (path, file)) in walk::everything(&top, glob.visible).enumerate() {
            if glob
                .most_looked_at
                .is_some_and(|most| looked_at >= most || started.elapsed() > LOOKING_FOR)
            {
                stopped = true;
                break;
            }
            if !file || !pattern.matches(path.strip_prefix(&top).unwrap_or(&path)) {
                continue;
            }
            let Some((_, _, stat)) = plain(&root, &path) else {
                continue;
            };
            let when = modified(&stat);
            let settled = cut_off.is_none_or(|cut_off| when <= cut_off);
            if since.is_none_or(|since| when > since) && settled {
                files.push((when, relative(root.path, &path)));
            }
        }
        // Newest first: what changed recently is what a task is usually about.
        files.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        let total = u64::try_from(files.len()).unwrap_or(u64::MAX);
        files.truncate(glob.limit);
        Ok(Globbed {
            path: shown(&found.names),
            files: files.into_iter().map(|(_, path)| path).collect(),
            total,
            stopped,
            cut_off_ns: cut_off.map(nanos_since_1970),
        })
    })
    .await
}

/// `time` in nanoseconds since 1970; none before it.
fn nanos_since_1970(time: SystemTime) -> u64 {
    let since = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    u64::try_from(since.as_nanos()).unwrap_or(u64::MAX)
}

fn modified(stat: &Stat) -> SystemTime {
    let nanos = u32::try_from(count(stat.st_mtime_nsec)).unwrap_or(0);
    SystemTime::UNIX_EPOCH + Duration::new(count(stat.st_mtime), nanos)
}

pub(crate) async fn search(root: &Path, search: &Search) -> Result<Searched, ComputerError> {
    let filter = search
        .glob
        .as_deref()
        .map(Pattern::new)
        .transpose()
        .map_err(|refused| ComputerError::Refused(refused.into()))?;
    let (regex, as_text) = compile(&search.pattern, search.ignore_case)?;
    let (root, search) = (root.to_owned(), search.clone());
    blocking(move || {
        let root = Root::open(&root)?;
        let found = resolve(&root, &search.path)?;
        let top = root.path.join(found.names.iter().collect::<PathBuf>());
        let mut answer = Searched {
            path: shown(&found.names),
            matches: Vec::new(),
            more: false,
            secrets: Vec::new(),
            as_text,
        };
        scan(
            &root,
            &top,
            &regex,
            filter.as_ref(),
            &mut answer,
            (search.limit, search.line_chars),
        );
        Ok(answer)
    })
    .await
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
            .map_err(|e| {
                ComputerError::Refused(format!("The pattern could not be searched: {e}").into())
            }),
    }
}

fn scan(
    root: &Root<'_>,
    top: &Path,
    regex: &Regex,
    filter: Option<&Pattern>,
    found: &mut Searched,
    (limit, line_chars): (usize, usize),
) {
    for path in walk::files(top) {
        let relative_to_start = path.strip_prefix(top).unwrap_or(&path);
        if filter.is_some_and(|filter| !filter.matches(relative_to_start)) {
            continue;
        }
        if is_secret_file(&path) {
            if plain(root, &path).is_some() && found.secrets.len() < limit {
                found.secrets.push(relative(root.path, &path));
            }
            continue;
        }
        let Some(text) = searchable(root, &path) else {
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
                path: relative(root.path, &path),
                line: u64::try_from(index.saturating_add(1)).unwrap_or(u64::MAX),
                text: clip(line, line_chars),
            });
        }
    }
}

/// The file's text, unless it is too big, unreadable or binary, read through a lookup from
/// the root with no link along the way.
fn searchable(root: &Root<'_>, path: &Path) -> Option<String> {
    let (folder, name, _) = plain(root, path)?;
    let file = open_to_read(folder.as_fd(), &name).ok()?;
    if file.metadata().ok()?.len() > MAX_SEARCHED_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_SEARCHED_BYTES).read_to_end(&mut bytes).ok()?;
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

#[cfg(test)]
mod tests;
