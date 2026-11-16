//! Whether a file a turn made holds one of the person's secrets (`specs/session.md`, "Files it
//! made"): each value as it is written, and as XML writes it, in the file's bytes and inside
//! each part a zip file (an Office file, an e-book) keeps deflated. Parts are found by their
//! local headers and inflated a piece at a time, never more than [`MOST_INFLATED`] in all, so a
//! file that inflates past it can't be told, and is not kept. Nothing is read of the zip's own
//! record of its parts, so no size it claims is trusted.

use std::io::Read as _;

use flate2::read::DeflateDecoder;

/// The most a file's deflated parts are inflated to, in all: more than the parts of any file a
/// thread keeps (10 MB) hold, but for a crafted one.
const MOST_INFLATED: u64 = 64 * 1024 * 1024;

/// How much of a part is inflated at a time.
const PIECE: usize = 64 * 1024;

/// What starts each part of a zip file.
const LOCAL_HEADER: [u8; 4] = *b"PK\x03\x04";

/// How a part's data is compressed when it is deflated.
const DEFLATED: u16 = 8;

/// What looking through a file for the person's secrets found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scanned {
    /// None of their values.
    Clean,
    /// One of them, as written or inside a part.
    Holds,
    /// Its parts inflate past what is looked through, so it can't be told.
    Unchecked,
}

/// Whether `bytes` hold any of `values`, as written or as XML writes them, or inside a
/// deflated part. With no values, nothing is inflated.
pub(crate) fn scanned(bytes: &[u8], values: &[Vec<u8>]) -> Scanned {
    let mut needles: Vec<Vec<u8>> = Vec::new();
    for value in values.iter().filter(|value| !value.is_empty()) {
        for needle in [value.clone(), escaped(value)] {
            if !needles.contains(&needle) {
                needles.push(needle);
            }
        }
    }
    if needles.is_empty() {
        return Scanned::Clean;
    }
    if needles.iter().any(|needle| holds(bytes, needle)) {
        return Scanned::Holds;
    }
    let mut inflated = 0;
    let starts = bytes
        .windows(LOCAL_HEADER.len())
        .enumerate()
        .filter(|(_, window)| *window == LOCAL_HEADER)
        .map(|(at, _)| at);
    for at in starts {
        match part(bytes, at, &needles, &mut inflated) {
            Scanned::Clean => {}
            found => return found,
        }
    }
    Scanned::Clean
}

/// Whether the part whose local header starts at `at` in `bytes`, when it is deflated, holds
/// one of `needles`, counting what it inflates to in `inflated`. A part stored as it is was
/// looked through with the file's bytes; one that does not inflate is no part.
fn part(bytes: &[u8], at: usize, needles: &[Vec<u8>], inflated: &mut u64) -> Scanned {
    let field = |offset: usize| {
        let start = at.checked_add(offset)?;
        let two = bytes.get(start..start.checked_add(2)?)?;
        Some(u16::from_le_bytes(two.try_into().ok()?))
    };
    let (Some(method), Some(name), Some(extra)) = (field(8), field(26), field(28)) else {
        return Scanned::Clean;
    };
    let start = at
        .saturating_add(30)
        .saturating_add(usize::from(name))
        .saturating_add(usize::from(extra));
    let Some(data) = bytes.get(start..).filter(|_| method == DEFLATED) else {
        return Scanned::Clean;
    };
    let longest = needles.iter().map(Vec::len).max().unwrap_or(0);
    let mut decoder = DeflateDecoder::new(data);
    let mut piece = vec![0; PIECE];
    let mut window = Vec::with_capacity(PIECE.saturating_add(longest));
    loop {
        let read = match decoder.read(&mut piece) {
            Ok(0) | Err(_) => return Scanned::Clean,
            Ok(read) => read,
        };
        *inflated = inflated.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        if *inflated > MOST_INFLATED {
            return Scanned::Unchecked;
        }
        window.extend_from_slice(piece.get(..read).unwrap_or_default());
        if needles.iter().any(|needle| holds(&window, needle)) {
            return Scanned::Holds;
        }
        // What could be the start of a value stays for the next piece.
        let kept = window.len().saturating_sub(longest.saturating_sub(1));
        window.drain(..kept);
    }
}

/// Whether `needle` is in `bytes`.
fn holds(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

/// `value` as XML writes it in text and attributes.
fn escaped(value: &[u8]) -> Vec<u8> {
    let mut written = Vec::with_capacity(value.len());
    for byte in value {
        match byte {
            b'&' => written.extend_from_slice(b"&amp;"),
            b'<' => written.extend_from_slice(b"&lt;"),
            b'>' => written.extend_from_slice(b"&gt;"),
            b'"' => written.extend_from_slice(b"&quot;"),
            b'\'' => written.extend_from_slice(b"&apos;"),
            other => written.push(*other),
        }
    }
    written
}
