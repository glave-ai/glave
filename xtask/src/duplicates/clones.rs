//! Copies of code across sources: each source read as lines of code, every run of `window` of
//! them hashed, and each two places with the same run followed as far as both go on alike; the
//! places of one copied text gathered together.

use std::collections::{BTreeMap, BTreeSet};

/// A copy of code: the fingerprint of its text, its lines of code, and each place it is, as a
/// path and the line it starts on.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Copied {
    pub(crate) fingerprint: u64,
    pub(crate) lines: usize,
    pub(crate) places: Vec<(String, usize)>,
}

impl Copied {
    /// Its entry in the baseline: the fingerprint, its lines, and the file of each place.
    pub(crate) fn entry(&self) -> String {
        let files: Vec<&str> = self.places.iter().map(|(path, _)| path.as_str()).collect();
        format!(
            "{:016x} {} {}",
            self.fingerprint,
            self.lines,
            files.join(" ")
        )
    }

    /// Its places, as `path:line` each.
    pub(crate) fn places(&self) -> String {
        let places: Vec<String> = self
            .places
            .iter()
            .map(|(path, line)| format!("{path}:{line}"))
            .collect();
        places.join(", ")
    }
}

/// A line of code: its number in its file from 1, the hash of its text, and its tokens.
struct Line {
    number: usize,
    hash: u64,
    tokens: usize,
}

/// A place in the sources: which source, and which of its lines of code.
type At = (usize, usize);

/// The copies of `window` or more lines of code, and `least_tokens` or more tokens, among
/// `sources` (each a path and its text).
pub(crate) fn find(
    sources: &[(String, String)],
    window: usize,
    least_tokens: usize,
) -> Vec<Copied> {
    let code: Vec<Vec<Line>> = sources.iter().map(|(_, text)| lines(text)).collect();
    let hash = |(source, line): At| code.get(source)?.get(line).map(|line| line.hash);
    let mut runs: BTreeMap<u64, Vec<At>> = BTreeMap::new();
    for (source, lines) in code.iter().enumerate() {
        for (start, run) in lines.windows(window).enumerate() {
            let key = fnv(run.iter().flat_map(|line| line.hash.to_le_bytes()));
            runs.entry(key).or_default().push((source, start));
        }
    }
    let mut copies: BTreeMap<u64, (usize, BTreeSet<At>)> = BTreeMap::new();
    for places in runs.values().filter(|places| places.len() > 1) {
        for (index, &a) in places.iter().enumerate() {
            for &b in places.iter().skip(index + 1) {
                let Some(lines) = alike(a, b, window, &hash) else {
                    continue;
                };
                let text = || (a.1..a.1 + lines).filter_map(|line| code.get(a.0)?.get(line));
                if text().map(|line| line.tokens).sum::<usize>() < least_tokens {
                    continue;
                }
                let fingerprint = fnv(text().flat_map(|line| line.hash.to_le_bytes()));
                let copy = copies
                    .entry(fingerprint)
                    .or_insert((lines, BTreeSet::new()));
                copy.1.extend([a, b]);
            }
        }
    }
    let place = |(source, line): At| {
        let path = sources.get(source).map(|(path, _)| path.clone())?;
        Some((path, code.get(source)?.get(line)?.number))
    };
    let mut found: Vec<Copied> = copies
        .into_iter()
        .map(|(fingerprint, (lines, places))| Copied {
            fingerprint,
            lines,
            places: places.into_iter().filter_map(place).collect(),
        })
        .collect();
    found.sort_by_key(Copied::entry);
    found
}

/// How many lines of code the copy that starts at both `a` and `b` (`a` first) has: `window`
/// and as many more as both go on alike, none past where the second starts when they share a
/// file. None when their lines differ, when they overlap, or when the copy starts earlier.
fn alike(a: At, b: At, window: usize, hash: &impl Fn(At) -> Option<u64>) -> Option<usize> {
    let same = |offset: usize| {
        hash((a.0, a.1 + offset)).is_some_and(|line| Some(line) == hash((b.0, b.1 + offset)))
    };
    let room = |lines: usize| a.0 != b.0 || a.1 + lines <= b.1;
    if !room(window) || !(0..window).all(same) {
        return None;
    }
    let earlier = a.1.checked_sub(1).zip(b.1.checked_sub(1));
    if earlier.is_some_and(|(before_a, before_b)| {
        hash((a.0, before_a)).is_some_and(|line| Some(line) == hash((b.0, before_b)))
    }) {
        return None;
    }
    let mut lines = window;
    while room(lines + 1) && same(lines) {
        lines += 1;
    }
    Some(lines)
}

/// The lines of code in `text`: each trimmed, blank lines, lines of brackets alone, comments,
/// and imports, re-exports and module declarations left out, since those are meant to look
/// alike.
fn lines(text: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut importing = false;
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if importing || imports(line) {
            importing = !line.ends_with(';');
            continue;
        }
        if line.is_empty()
            || line.chars().all(|c| "{}()[];,".contains(c))
            || comment(line)
            || declares_module(line)
        {
            continue;
        }
        lines.push(Line {
            number: index + 1,
            hash: fnv(line.bytes()),
            tokens: tokens(line),
        });
    }
    lines
}

/// Whether `line` starts an import or a re-export, which runs to its `;`.
fn imports(line: &str) -> bool {
    let line = unscoped(line);
    ["use ", "import ", "export * from ", "export {"]
        .iter()
        .any(|start| line.starts_with(start))
}

/// Whether `line` declares a module: `mod name;`, or the line that opens one.
fn declares_module(line: &str) -> bool {
    unscoped(line).starts_with("mod ")
}

/// `line` without the visibility it starts with (`pub`, `pub(crate)`).
fn unscoped(line: &str) -> &str {
    match line.strip_prefix("pub") {
        Some(rest) if rest.starts_with('(') => rest.split_once(") ").map_or(line, |(_, rest)| rest),
        Some(rest) => rest.strip_prefix(' ').unwrap_or(line),
        None => line,
    }
}

/// Whether `line` is a comment, or a line inside a block comment.
fn comment(line: &str) -> bool {
    line.starts_with("//")
        || line.starts_with("/*")
        || line.starts_with("* ")
        || line.starts_with("*/")
        || line == "*"
}

/// The tokens of `line`: its words, and each mark that is not one.
fn tokens(line: &str) -> usize {
    let mut tokens = 0;
    let mut in_word = false;
    for c in line.chars() {
        let word = c.is_alphanumeric() || c == '_';
        if (word && !in_word) || (!word && !c.is_whitespace()) {
            tokens += 1;
        }
        in_word = word;
    }
    tokens
}

/// The 64-bit FNV-1a hash of `bytes`: the same on every machine and every Rust, so a
/// fingerprint written down stays true.
fn fnv(bytes: impl IntoIterator<Item = u8>) -> u64 {
    bytes.into_iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}
