//! An Office file taken apart to edit in place, and put back together: `glave doc unpack`
//! writes each part to a folder, its XML laid out an element to a line, and `glave doc pack`
//! zips the folder back into a file Office opens, each part checked first.
//!
//! Unpacking writes down the parts' order, and whether each was stored or deflated, in a file
//! in the folder ([`ORDER`]), so packing puts them back as they were, with
//! `[Content_Types].xml` first; parts added since go after them, deflated.

mod checks;

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::{Cursor, Read, Write as _};
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use crate::cells::listed;
use crate::error::OfficeError;
use crate::layout::{condensed, spread};
use crate::package::{CONTENT_TYPES, Kind, Package};
use crate::xml;

/// The file in an unpacked folder that keeps its parts' order and how each was zipped.
const ORDER: &str = ".glave-parts";

/// The most bytes of parts unpacked from one file, or packed into one.
const MAX_PARTS: u64 = 512 * 1024 * 1024;

/// The most files a folder is packed from.
const MAX_COUNT: usize = 10_000;

/// The parts named when unpacking says where things are, of however many there are.
const NAMED: usize = 12;

/// Writes the parts of the Office file in `bytes`, at `file`, into `folder`, which must be new
/// or empty, and says where its text is.
pub(crate) fn unpack(bytes: &[u8], file: &Path, folder: &Path) -> Result<String, OfficeError> {
    let shown = file.display().to_string();
    let mut zip =
        ZipArchive::new(Cursor::new(bytes)).map_err(|_| OfficeError::NotAPackage(shown.clone()))?;
    if zip.index_for_name(CONTENT_TYPES).is_none() {
        return Err(OfficeError::NotAPackage(shown));
    }
    if std::fs::read_dir(folder).is_ok_and(|mut entries| entries.next().is_some()) {
        return Err(OfficeError::Parts(format!(
            "{} is not empty: unpack into a new folder",
            folder.display()
        )));
    }
    make(folder)?;
    let mut order =
        format!("# The parts of {shown} in order, each stored or deflated, for glave doc pack.\n");
    let (mut room, mut count) = (MAX_PARTS, 0);
    for index in 0..zip.len() {
        let damaged = |error: zip::result::ZipError| OfficeError::Damaged(error.to_string());
        let mut part = zip.by_index(index).map_err(damaged)?;
        let name = part.name().map_err(damaged)?.into_owned();
        if part.is_dir() {
            continue;
        }
        let Some(path) = inside(folder, &name) else {
            return Err(OfficeError::Parts(format!(
                "{shown} has a part named {name:?}, which would be written outside the folder"
            )));
        };
        let mut bytes = Vec::new();
        (&mut part)
            .take(room + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| OfficeError::Damaged(format!("{name}: {error}")))?;
        room = taken(room, bytes.len(), &shown)?;
        let method = match part.compression() {
            CompressionMethod::Stored => "stored",
            _ => "deflated",
        };
        if let Some(parent) = path.parent() {
            make(parent)?;
        }
        let written = match std::str::from_utf8(&bytes) {
            Ok(text) if is_xml(&name) => spread(text).into_bytes(),
            _ => bytes,
        };
        std::fs::write(&path, written).map_err(|error| OfficeError::Io {
            action: "write",
            path,
            error,
        })?;
        let _ = writeln!(order, "{method} {name}");
        count += 1;
    }
    let listing = folder.join(ORDER);
    std::fs::write(&listing, order).map_err(|error| OfficeError::Io {
        action: "write",
        path: listing,
        error,
    })?;
    let mut said = format!("Unpacked {shown} into {}: {count} parts.", folder.display());
    if let Some(guide) = guide(bytes) {
        let _ = write!(said, " {guide}");
    }
    let _ = write!(
        said,
        "\nWhen it is edited, pack it back: glave doc pack {} {shown} --replace",
        folder.display()
    );
    Ok(said)
}

/// Where the text of the Office file in `bytes` is, in a sentence.
fn guide(bytes: &[u8]) -> Option<String> {
    let mut package = Package::open(bytes)?;
    let (main, kind) = package.main().ok()??;
    Some(match kind {
        Kind::Word => format!("The text is in {main}."),
        Kind::PowerPoint => {
            let slides = package.listed(&main, "p:sldIdLst").ok()?;
            let named: Vec<String> = slides.into_iter().map(|(_, part)| part).collect();
            let shown: Vec<String> = named.iter().take(NAMED).cloned().collect();
            format!(
                "The slides, in the deck's order: {}.",
                listed(&shown, named.len())
            )
        }
        Kind::Workbook => {
            let sheets = package.listed(&main, "sheets").ok()?;
            let shown: Vec<String> = sheets
                .iter()
                .take(NAMED)
                .map(|(name, part)| format!("{name} is {part}"))
                .collect();
            format!(
                "The sheets: {}. A cell marked t=\"s\" keeps its text in xl/sharedStrings.xml.",
                listed(&shown, sheets.len())
            )
        }
    })
}

/// Makes `folder` and the folders it is in.
fn make(folder: &Path) -> Result<(), OfficeError> {
    std::fs::create_dir_all(folder).map_err(|error| OfficeError::Io {
        action: "make the folder",
        path: folder.to_path_buf(),
        error,
    })
}

/// The path of the part named `name` in `folder`, unless its name would take it out of it.
fn inside(folder: &Path, name: &str) -> Option<PathBuf> {
    let mut path = folder.to_path_buf();
    for segment in name.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return None;
        }
        if segment.contains(['\\', ':', '\0']) {
            return None;
        }
        path.push(segment);
    }
    Some(path)
}

/// Whether a part holds XML to lay out and check: `.xml` and `.rels` parts. VML drawings are
/// left alone: Office writes them more as HTML than as XML.
fn is_xml(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, extension)| {
        extension.eq_ignore_ascii_case("xml") || extension.eq_ignore_ascii_case("rels")
    })
}

/// The room left of [`MAX_PARTS`] once `used` more bytes are taken from `room`.
fn taken(room: u64, used: usize, what: &str) -> Result<u64, OfficeError> {
    room.checked_sub(u64::try_from(used).unwrap_or(u64::MAX))
        .ok_or_else(|| {
            OfficeError::Parts(format!(
                "{what} holds more than the {MAX_PARTS} bytes glave unpacks or packs"
            ))
        })
}

/// A part to pack: its name, its bytes, and whether to store it rather than deflate it.
struct Part {
    name: String,
    bytes: Vec<u8>,
    stored: bool,
}

/// Zips the parts in `folder` into `out`, unless a file is there and `replace` is not given:
/// each XML part checked, and drawn back together as it was before it was unpacked.
pub(crate) fn pack(folder: &Path, out: &Path, replace: bool) -> Result<String, OfficeError> {
    if !replace && std::fs::symlink_metadata(out).is_ok() {
        return Err(OfficeError::PackedOver(out.to_path_buf()));
    }
    let shown = folder.display().to_string();
    let mut found = BTreeSet::new();
    files(folder, "", &mut found)?;
    if !found.contains(CONTENT_TYPES) {
        return Err(OfficeError::Parts(format!(
            "{shown} has no {CONTENT_TYPES}, so it is not an unpacked Office file: unpack one \
             with glave doc unpack"
        )));
    }
    let mut parts = Vec::new();
    let mut room = MAX_PARTS;
    for (name, stored) in ordered(folder, found)? {
        let path = inside(folder, &name).unwrap_or_else(|| folder.join(&name));
        let bytes = std::fs::read(&path).map_err(|error| OfficeError::Io {
            action: "read",
            path,
            error,
        })?;
        room = taken(room, bytes.len(), &shown)?;
        let bytes = match String::from_utf8(bytes) {
            Ok(text) if is_xml(&name) => {
                xml::check(&text).map_err(|broken| OfficeError::Broken {
                    part: name.clone(),
                    broken: format!("{broken}. Nothing was packed: mend it, then pack again"),
                })?;
                condensed(&text).into_bytes()
            }
            Ok(text) => text.into_bytes(),
            Err(not_text) => not_text.into_bytes(),
        };
        parts.push(Part {
            name,
            bytes,
            stored,
        });
    }
    checks::tied(&parts)?;
    let zipped = zipped(&parts).map_err(|error| OfficeError::Parts(error.to_string()))?;
    std::fs::write(out, zipped).map_err(|error| OfficeError::Io {
        action: "write",
        path: out.to_path_buf(),
        error,
    })?;
    let out = out.display();
    Ok(format!(
        "Packed {shown} into {out}: {} parts. Read it back to check it: glave doc read {out}",
        parts.len()
    ))
}

/// The parts `found` in `folder`, in the order unpacking wrote down, each with whether it was
/// stored; those added since after them, deflated; `[Content_Types].xml` first of all.
fn ordered(folder: &Path, mut found: BTreeSet<String>) -> Result<Vec<(String, bool)>, OfficeError> {
    let listing = folder.join(ORDER);
    let written = match std::fs::read_to_string(&listing) {
        Ok(written) => written,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(OfficeError::Io {
                action: "read",
                path: listing,
                error,
            });
        }
    };
    let mut ordered = Vec::new();
    for line in written.lines().filter(|line| !line.starts_with('#')) {
        if let Some((method, name)) = line.split_once(' ')
            && found.remove(name)
        {
            ordered.push((name.to_owned(), method == "stored"));
        }
    }
    ordered.extend(found.into_iter().map(|name| (name, false)));
    ordered.sort_by_key(|(name, _)| name != CONTENT_TYPES);
    Ok(ordered)
}

/// Every file under `folder`, named as a part is (`word/document.xml`), but for hidden ones
/// such as the order unpacking wrote: of a package's own, only `.rels` starts with a dot.
fn files(folder: &Path, prefix: &str, found: &mut BTreeSet<String>) -> Result<(), OfficeError> {
    let io = |error| OfficeError::Io {
        action: "read",
        path: folder.to_path_buf(),
        error,
    };
    for entry in std::fs::read_dir(folder).map_err(io)? {
        let entry = entry.map_err(io)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') && name != ".rels" {
            continue;
        }
        let part = format!("{prefix}{name}");
        let kind = entry.file_type().map_err(io)?;
        if kind.is_symlink() {
            return Err(OfficeError::Parts(format!(
                "{part} is a link: a packed file holds only the folder's own files"
            )));
        }
        if kind.is_dir() {
            files(&entry.path(), &format!("{part}/"), found)?;
        } else {
            found.insert(part);
        }
        if found.len() > MAX_COUNT {
            return Err(OfficeError::Parts(format!(
                "the folder holds more than the {MAX_COUNT} files glave packs"
            )));
        }
    }
    Ok(())
}

/// `parts` zipped in order, as Office zips its files.
fn zipped(parts: &[Part]) -> zip::result::ZipResult<Vec<u8>> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for part in parts {
        let method = if part.stored {
            CompressionMethod::Stored
        } else {
            CompressionMethod::Deflated
        };
        let options = SimpleFileOptions::default()
            .compression_method(method)
            .last_modified_time(DateTime::default());
        zip.start_file(part.name.as_str(), options)?;
        zip.write_all(&part.bytes)?;
    }
    Ok(zip.finish()?.into_inner())
}

#[cfg(test)]
mod tests;
