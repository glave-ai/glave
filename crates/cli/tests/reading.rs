//! A reader of a file a turn made is held to its memory on every computer (`reading.rs`): a
//! workbook that takes more memory to read than a reader may have ends the reader with what it
//! held under 2 GB, as the operating system measures it. glave-office reads an xlsx or xlsb
//! workbook within limits of its own, so the workbook is an OpenDocument spreadsheet, which
//! calamine reads whole as it opens it: a few cells far apart, which it holds as a range of
//! every cell between them, just under its own cap of 100 million (some 3 GB).

use std::time::Duration;

use flate2::Crc;
use glave_ports::testing::read_program;
use glave_process::{Processes, Program};

/// What the program is started with to read a file, as the server starts it.
const READER_ARG: &str = "--glave-read-made";

const ODS: &str = "application/vnd.oasis.opendocument.spreadsheet";

/// A row holding a number at its first and last columns.
const ENDS: &str = r#"<table:table-row><table:table-cell office:value-type="float" office:value="1"/><table:table-cell table:number-columns-repeated="16382"/><table:table-cell office:value-type="float" office:value="1"/></table:table-row>"#;

/// What an OpenDocument file lists of its parts: here nothing, which calamine takes as a file
/// with no password.
const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"/>"#;

/// An OpenDocument spreadsheet holding numbers at its four corners, 6,100 rows and 16,384
/// columns apart, every part stored as it is.
fn spread_workbook() -> Vec<u8> {
    let empty = r#"<table:table-row table:number-rows-repeated="6098"><table:table-cell table:number-columns-repeated="16384"/></table:table-row>"#;
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" office:version="1.2"><office:body><office:spreadsheet><table:table table:name="Budget">{ENDS}{empty}{ENDS}</table:table></office:spreadsheet></office:body></office:document-content>"#
    );
    let parts: Vec<(&str, u16, u32, Vec<u8>, usize)> = [
        ("mimetype", ODS),
        ("META-INF/manifest.xml", MANIFEST),
        ("content.xml", &content),
    ]
    .iter()
    .map(|(name, text)| {
        let mut crc = Crc::new();
        crc.update(text.as_bytes());
        (*name, 0, crc.sum(), text.as_bytes().to_vec(), text.len())
    })
    .collect();
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
    let workbook = spread_workbook();
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
    reader.arg(READER_ARG).arg(ODS);
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
