//! Whether a file a turn made holds one of the person's secrets (`specs/session.md`, "Files it
//! made"): each value as it is written, and as XML writes it, in the file's bytes and inside
//! each part a zip file (an Office file, an e-book) keeps deflated. Parts are found by their
//! local headers, at most [`MOST_PARTS`] of them, and inflated a piece at a time by one inflater
//! reset for each; what each takes in and gives out counts against [`MOST_WORK`] in all, so a
//! file that takes more can't be told, and is not kept. The work is linear in what it counts:
//! each value is looked for with one prepared search (`memmem`). Nothing is read of the zip's
//! own record of its parts, so no size it claims is trusted.

use flate2::{Decompress, FlushDecompress, Status};
use memchr::memmem::{self, Finder};

/// The most bytes of parts taken in and inflated to, in all: more than the parts of any file a
/// thread keeps (10 MB) hold, a workbook's included, but for a crafted one.
const MOST_WORK: u64 = 256 * 1024 * 1024;

/// The most parts a file is looked into: more than any real Office file or e-book has.
const MOST_PARTS: usize = 65_536;

/// How much of a part is inflated at a time.
const PIECE: usize = 64 * 1024;

/// What starts each part of a zip file.
const LOCAL_HEADER: &[u8] = b"PK\x03\x04";

/// How a part's data is compressed when it is deflated.
const DEFLATED: u16 = 8;

/// What looking through a file for the person's secrets found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scanned {
    /// None of their values.
    Clean,
    /// One of them, as written or inside a part.
    Holds,
    /// Its parts take more than is looked through, so it can't be told.
    Unchecked,
}

/// What a scan shares from part to part: the searches, one inflater, its pieces, and the work
/// done so far.
struct Scan<'a> {
    finders: Vec<Finder<'a>>,
    /// The longest value looked for, less one byte: what stays from one piece for the next.
    overlap: usize,
    inflater: Decompress,
    piece: Vec<u8>,
    window: Vec<u8>,
    work: u64,
    /// The most work it may do.
    most: u64,
}

/// Whether `bytes` hold any of `values`, as written or as XML writes them, or inside a
/// deflated part. With no values, nothing is inflated.
pub(crate) fn scanned(bytes: &[u8], values: &[Vec<u8>]) -> Scanned {
    scanned_within(bytes, values, MOST_WORK)
}

/// [`scanned`], doing at most `most` bytes of work in its parts.
pub(crate) fn scanned_within(bytes: &[u8], values: &[Vec<u8>], most: u64) -> Scanned {
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
    let longest = needles.iter().map(Vec::len).max().unwrap_or(0);
    let mut scan = Scan {
        finders: needles.iter().map(|needle| Finder::new(needle)).collect(),
        overlap: longest.saturating_sub(1),
        inflater: Decompress::new(false),
        piece: vec![0; PIECE],
        window: Vec::with_capacity(PIECE.saturating_add(longest)),
        work: 0,
        most,
    };
    if scan.holds(bytes) {
        return Scanned::Holds;
    }
    for (count, at) in memmem::find_iter(bytes, LOCAL_HEADER).enumerate() {
        if count >= MOST_PARTS {
            return Scanned::Unchecked;
        }
        match scan.part(bytes, at) {
            Scanned::Clean => {}
            found => return found,
        }
    }
    Scanned::Clean
}

impl Scan<'_> {
    /// Whether one of the values is in `bytes`.
    fn holds(&self, bytes: &[u8]) -> bool {
        self.finders
            .iter()
            .any(|finder| finder.find(bytes).is_some())
    }

    /// Whether the part whose local header starts at `at` in `bytes`, when it is deflated, holds
    /// one of the values. A part stored as it is was looked through with the file's bytes; one
    /// that does not inflate is no part.
    fn part(&mut self, bytes: &[u8], at: usize) -> Scanned {
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
        let Some(mut input) = bytes.get(start..).filter(|_| method == DEFLATED) else {
            return Scanned::Clean;
        };
        self.inflater.reset(false);
        self.window.clear();
        loop {
            let (taken, given) = (self.inflater.total_in(), self.inflater.total_out());
            let status = self
                .inflater
                .decompress(input, &mut self.piece, FlushDecompress::None);
            let taken = self.inflater.total_in().saturating_sub(taken);
            let given = self.inflater.total_out().saturating_sub(given);
            self.work = self.work.saturating_add(taken).saturating_add(given);
            if self.work > self.most {
                return Scanned::Unchecked;
            }
            input = input
                .get(usize::try_from(taken).unwrap_or(usize::MAX)..)
                .unwrap_or_default();
            let inflated = self
                .piece
                .get(..usize::try_from(given).unwrap_or(0))
                .unwrap_or_default();
            self.window.extend_from_slice(inflated);
            if self.holds(&self.window) {
                return Scanned::Holds;
            }
            // What could be the start of a value stays for the next piece.
            let kept = self.window.len().saturating_sub(self.overlap);
            self.window.drain(..kept);
            let moved = taken > 0 || given > 0;
            if !moved || !matches!(status, Ok(Status::Ok | Status::BufError)) {
                return Scanned::Clean;
            }
        }
    }
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
