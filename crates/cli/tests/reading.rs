//! A reader of a file a turn made is held to its memory on every computer (`reading.rs`): a
//! workbook whose strings take more and more memory as they are read, past what a reader may
//! have, ends the reader with what it held under 2 GB, as the operating system measures it.

use std::time::Duration;

use flate2::{Compress, Compression, Crc, FlushCompress};
use glave_ports::testing::read_program;
use glave_process::{Processes, Program};

/// What the program is started with to read a file, as the server starts it.
const READER_ARG: &str = "--glave-read-made";

const XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

/// The parts of a workbook beside its strings, stored as they are.
const PARTS: [(&str, &str); 5] = [
    (
        "[Content_Types].xml",
        r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/></Types>"#,
    ),
    (
        "_rels/.rels",
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
    ),
    (
        "xl/workbook.xml",
        r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Budget" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
    ),
    (
        "xl/_rels/workbook.xml.rels",
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#,
    ),
    (
        "xl/worksheets/sheet1.xml",
        r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c></row></sheetData></worksheet>"#,
    ),
];

/// Megabytes of strings the workbook's strings part inflates to: more than a reader may hold,
/// less than 4 GB, should it hold them all.
const STRINGS_MB: usize = 2_000;

/// A deflated stream of `head`, then `times` copies of `block`, then `tail`, made cheaply:
/// each flushed whole, needing nothing before it, so its copies are copies of its bytes.
fn deflated(head: &[u8], block: &[u8], times: usize, tail: &[u8]) -> Vec<u8> {
    let mut compress = Compress::new(Compression::best(), false);
    let mut flushed = |input: &[u8], flush| {
        let mut out = Vec::with_capacity(input.len() / 8 + 1024);
        compress.compress_vec(input, &mut out, flush).unwrap();
        out
    };
    let (first, middle) = (
        flushed(head, FlushCompress::Full),
        flushed(block, FlushCompress::Full),
    );
    let mut stream = first;
    stream.extend_from_slice(&middle.repeat(times));
    stream.extend_from_slice(&flushed(tail, FlushCompress::Finish));
    stream
}

/// A workbook whose strings part holds strings of 64 KB each, `STRINGS_MB` megabytes of them,
/// read one after another into memory; its record of that part's checksum is wrong, which a
/// reader finds only at its end.
fn growing_workbook() -> Vec<u8> {
    let head = br#"<?xml version="1.0" encoding="UTF-8"?><sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#;
    let text = vec![b'a'; 65_536];
    let entry = [b"<si><t>".as_slice(), &text, b"</t></si>"].concat();
    let block = entry.repeat((1 << 20) / entry.len());
    let tail = b"</sst>";
    let strings = deflated(head, &block, STRINGS_MB, tail);
    let inflated = head.len() + block.len() * STRINGS_MB + tail.len();
    let mut parts: Vec<(&str, u16, u32, Vec<u8>, usize)> = PARTS
        .iter()
        .map(|(name, text)| {
            let mut crc = Crc::new();
            crc.update(text.as_bytes());
            (*name, 0, crc.sum(), text.as_bytes().to_vec(), text.len())
        })
        .collect();
    parts.push(("xl/sharedStrings.xml", 8, 0, strings, inflated));
    zipped(&parts)
}

/// A zip of `parts`, each its name, how it is compressed, its checksum, its data and the size
/// it inflates to.
fn zipped(parts: &[(&str, u16, u32, Vec<u8>, usize)]) -> Vec<u8> {
    let (mut body, mut central) = (Vec::new(), Vec::new());
    let u32_of = |size: usize| u32::try_from(size).unwrap_or(u32::MAX);
    for (name, method, crc, data, inflated) in parts {
        let offset = u32_of(body.len());
        let sizes = [u32_of(data.len()), u32_of(*inflated)];
        let named = u16::try_from(name.len()).unwrap();
        body.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        for field in [20_u16, 0, *method, 0, 0] {
            body.extend_from_slice(&field.to_le_bytes());
        }
        for field in [*crc, sizes[0], sizes[1]] {
            body.extend_from_slice(&field.to_le_bytes());
        }
        body.extend_from_slice(&named.to_le_bytes());
        body.extend_from_slice(&0_u16.to_le_bytes());
        body.extend_from_slice(name.as_bytes());
        body.extend_from_slice(data);
        central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        for field in [20_u16, 20, 0, *method, 0, 0] {
            central.extend_from_slice(&field.to_le_bytes());
        }
        for field in [*crc, sizes[0], sizes[1]] {
            central.extend_from_slice(&field.to_le_bytes());
        }
        for field in [named, 0, 0, 0, 0] {
            central.extend_from_slice(&field.to_le_bytes());
        }
        central.extend_from_slice(&0_u32.to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let count = u16::try_from(parts.len()).unwrap();
    let mut end = 0x0605_4b50_u32.to_le_bytes().to_vec();
    for field in [0_u16, 0, count, count] {
        end.extend_from_slice(&field.to_le_bytes());
    }
    end.extend_from_slice(&u32_of(central.len()).to_le_bytes());
    end.extend_from_slice(&u32_of(body.len()).to_le_bytes());
    end.extend_from_slice(&0_u16.to_le_bytes());
    [body, central, end].concat()
}

/// The most memory the program `report` measured held at once, in bytes, as `time` says it:
/// in bytes on macOS (`-l`), in kilobytes on Linux (`-v`).
fn most_held(report: &str) -> Option<u64> {
    report.lines().find_map(|line| {
        let line = line.trim();
        if let Some(bytes) = line.strip_suffix("maximum resident set size") {
            return bytes.trim().parse().ok();
        }
        let kilobytes = line.strip_prefix("Maximum resident set size (kbytes):")?;
        kilobytes.trim().parse::<u64>().ok().map(|kb| kb * 1024)
    })
}

#[test]
fn a_reader_that_needs_more_memory_than_it_may_have_ends_under_two_gigabytes() {
    let glave = env!("CARGO_BIN_EXE_glave");
    read_program(glave).unwrap();
    let workbook = growing_workbook();
    assert!(workbook.len() < 10 << 20, "{}", workbook.len());
    let time = std::path::Path::new("/usr/bin/time");
    let mut reader = if time.exists() {
        let mut timed = Program::new(time);
        timed.arg(if cfg!(target_os = "macos") {
            "-l"
        } else {
            "-v"
        });
        timed.arg(glave);
        timed
    } else {
        Program::new(glave)
    };
    reader.arg(READER_ARG).arg(XLSX);
    let processes = Processes::unrecorded();
    let read = glave_ports::block_on!(processes.output_with(
        &mut reader,
        &workbook,
        Duration::from_secs(120),
        1 << 20,
    ));
    let read = read.unwrap();
    let said = String::from_utf8_lossy(&read.stderr);
    assert!(!read.status.success(), "{said}");
    assert!(said.contains("memory allocation of"), "{said}");
    if time.exists() {
        let held = most_held(&said).unwrap();
        assert!(held < 2 << 30, "held {held} bytes: {said}");
    }
}
