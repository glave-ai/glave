//! The strings an xlsx or xlsb workbook shares among its cells, measured before calamine opens
//! it, since calamine reads them whole as it opens a workbook and, for xlsx, first reserves room
//! for as many as the part says it holds. calamine finds the part by name whatever its case or
//! slashes, under whichever folder the workbook's relationships name, so every part whose name
//! ends so is measured, together, as a stream.

use std::io::{BufRead, BufReader, Cursor, Read as _};

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use zip::ZipArchive;

use crate::error::OfficeError;

/// The most bytes the strings a workbook shares may take, unpacked, in all.
const MOST_BYTES: u64 = 128 * 1024 * 1024;

/// The most strings a workbook may say it shares: as many as [`MOST_BYTES`] can hold, each
/// taking at least 16 bytes of its part (`<si><t></t></si>`).
const MOST_STRINGS: u64 = MOST_BYTES / 16;

/// How the name of a part of shared strings ends, in lower case with forward slashes.
const ENDINGS: [&str; 2] = ["sharedstrings.xml", "sharedstrings.bin"];

/// Refuses a workbook whose shared strings take more than [`MOST_BYTES`] unpacked, or that says
/// it holds more than [`MOST_STRINGS`]. A file that is not a zip (xls) has none.
pub(crate) fn shared_strings(bytes: &[u8]) -> Result<(), OfficeError> {
    let Ok(mut zip) = ZipArchive::new(Cursor::new(bytes)) else {
        return Ok(());
    };
    let mut left = MOST_BYTES;
    for at in 0..zip.len() {
        let Ok(part) = zip.by_index(at) else {
            continue;
        };
        let name = part
            .name()
            .map(|name| name.replace('\\', "/").to_ascii_lowercase())
            .unwrap_or_default();
        if !ENDINGS.iter().any(|ending| name.ends_with(ending)) {
            continue;
        }
        let unpacked = measured(BufReader::new(part.take(left.saturating_add(1))))?;
        left = left.checked_sub(unpacked).ok_or(OfficeError::TooMuchText {
            most: MOST_BYTES / (1024 * 1024),
        })?;
    }
    Ok(())
}

/// How many bytes `part` unpacks to, refusing it when its `<sst>` says it holds more strings
/// than [`MOST_STRINGS`].
fn measured(part: impl BufRead) -> Result<u64, OfficeError> {
    let unreadable = |error: String| OfficeError::NotAWorkbook(error);
    let mut xml = Reader::from_reader(part);
    let mut buf = Vec::new();
    loop {
        let event = xml.read_event_into(&mut buf);
        match event {
            Ok(Event::Start(element) | Event::Empty(element))
                if element.local_name().as_ref() == "sst" =>
            {
                let count = element
                    .try_get_attribute("uniqueCount")
                    .map_err(|error| unreadable(error.to_string()))?;
                if let Some(count) = count {
                    let count = count.value.to_string();
                    let digits = count.trim_start_matches('+');
                    let said = digits
                        .bytes()
                        .all(|byte| byte.is_ascii_digit())
                        .then(|| digits.parse::<u64>().ok())
                        .flatten();
                    if said.is_none_or(|said| said > MOST_STRINGS) {
                        return Err(OfficeError::TooManyStrings {
                            said: count.chars().take(24).collect(),
                            most: MOST_STRINGS,
                        });
                    }
                }
                break;
            }
            // An xlsb part, or one that is not XML: only its size counts.
            Ok(Event::Eof) | Err(_) => break,
            Ok(_) => buf.clear(),
        }
    }
    let read = xml.buffer_position();
    let rest = std::io::copy(&mut xml.into_inner(), &mut std::io::sink())
        .map_err(|error| unreadable(error.to_string()))?;
    Ok(read.saturating_add(rest))
}
