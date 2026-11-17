use std::io::Write as _;

use flate2::write::DeflateEncoder;
use flate2::{Compress, Compression, FlushCompress};
use glave_event::{Event, decode};
use glave_ports::testing::any_secret;
use serde_json::{Value, json};

use super::scan::{Scanned, scanned, scanned_within};
use super::*;

/// A zip of one part, `name`, deflated, holding `content`, as an Office file keeps its XML:
/// only its local header, which is all that is read of it.
fn zipped(name: &str, content: &[u8]) -> Vec<u8> {
    let mut deflating = DeflateEncoder::new(Vec::new(), Compression::fast());
    deflating.write_all(content).unwrap();
    with_header(name, &deflating.finish().unwrap(), content.len())
}

/// `data`, a part named `name` deflated from `inflated` bytes, after its local header.
fn with_header(name: &str, data: &[u8], inflated: usize) -> Vec<u8> {
    let mut zip = b"PK\x03\x04".to_vec();
    for field in [20_u16, 0, 8, 0, 0, 0, 0] {
        zip.extend_from_slice(&field.to_le_bytes());
    }
    zip.extend_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
    zip.extend_from_slice(&u32::try_from(inflated).unwrap_or(u32::MAX).to_le_bytes());
    zip.extend_from_slice(&u16::try_from(name.len()).unwrap().to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(name.as_bytes());
    zip.extend_from_slice(data);
    zip
}

/// A deflated stream of `times` megabytes of zeros, made cheaply: one megabyte flushed whole,
/// which needs nothing before it, again and again, then a last empty block.
fn zeros(times: usize) -> Vec<u8> {
    let mut compress = Compress::new(Compression::fast(), false);
    let mut block = Vec::with_capacity(64 * 1024);
    compress
        .compress_vec(&vec![0; 1024 * 1024], &mut block, FlushCompress::Full)
        .unwrap();
    let mut stream = block.repeat(times);
    let mut end = Vec::with_capacity(64);
    compress
        .compress_vec(&[], &mut end, FlushCompress::Finish)
        .unwrap();
    stream.extend_from_slice(&end);
    stream
}

#[test]
fn a_secret_is_found_as_written_inside_a_deflated_part_and_as_xml_writes_it() {
    let secret = any_secret();
    let value = format!("{secret}&x").into_bytes();
    let values = [value.clone()];
    let mut written = b"%PDF-1.7 ".to_vec();
    written.extend_from_slice(&value);
    assert_eq!(scanned(&written, &values), Scanned::Holds);
    let xml = format!("<w:t>{secret}&amp;x</w:t>");
    assert_eq!(
        scanned(&zipped("word/document.xml", xml.as_bytes()), &values),
        Scanned::Holds
    );
    // Across the edge of two pieces it is inflated in.
    let mut long = vec![b' '; 64 * 1024 - 5];
    long.extend_from_slice(&value);
    assert_eq!(
        scanned(&zipped("word/document.xml", &long), &values),
        Scanned::Holds
    );
    let clean = zipped("word/document.xml", b"<w:t>Trip budget</w:t>");
    assert_eq!(scanned(&clean, &values), Scanned::Clean);
    assert_eq!(
        scanned(&written, &[]),
        Scanned::Clean,
        "no values, nothing to find"
    );
}

#[test]
fn a_part_that_takes_more_than_is_looked_through_can_t_be_told() {
    let values = [any_secret().into_bytes()];
    // A workbook's strings may inflate past 64 MB: they are still looked through.
    let large = with_header("xl/sharedStrings.xml", &zeros(70), 70 << 20);
    assert_eq!(scanned(&large, &values), Scanned::Clean);
    // Past what may be done, here a few megabytes, nothing can be told.
    let most = 4 << 20;
    let bomb = with_header("xl/sharedStrings.xml", &zeros(5), 5 << 20);
    assert_eq!(scanned_within(&bomb, &values, most), Scanned::Unchecked);
    assert_eq!(
        scanned_within(&bomb, &[], most),
        Scanned::Clean,
        "with no values, nothing is inflated"
    );
    // Empty blocks, which inflate to nothing, count as what they take in.
    let empty: Vec<u8> = [0_u8, 0, 0, 0xFF, 0xFF].repeat(1 << 20);
    let blocks = with_header("word/document.xml", &empty, 0);
    assert_eq!(scanned_within(&blocks, &values, most), Scanned::Unchecked);
    // More parts than any real file has.
    let parts = b"PK\x03\x04".repeat(65_537);
    assert_eq!(scanned(&parts, &values), Scanned::Unchecked);
    assert_eq!(scanned(&parts[..65_536 * 4], &values), Scanned::Clean);
}

/// The event of `kind` with `data` at `seq`, as a thread's log holds it, decoded.
fn event(seq: u64, kind: &str, data: &Value) -> Event {
    decode(glave_ports::testing::logged(seq, kind, data).unwrap()).unwrap()
}

#[test]
fn where_a_thread_stands_is_read_again_from_its_log() {
    let config = json!({"sha256": "9".repeat(64), "bytes": 2, "media_type": "application/json"});
    let started = json!({"model": {"provider": "p", "name": "m"}, "config": config});
    let call = |tool: &str| json!({"call_id": "call_1", "tool": tool, "arguments": {"json": {}}});
    let marked = json!({"outcome": "completed", "made_until_ns": 42});
    let log = [
        event(1, "turn.started", &started),
        event(2, "tool.call.requested", &call("bash")),
        event(3, "turn.ended", &marked),
        event(4, "turn.started", &started),
        event(5, "turn.ended", &json!({"outcome": "completed"})),
        event(6, "turn.started", &started),
        event(7, "tool.call.requested", &call("agent")),
    ];
    let (open, mark) = from_log(&log, false);
    let seq = |seq: u64| {
        log.iter()
            .find(|event| event.header.seq.get() == seq)
            .unwrap()
            .header
            .seq
    };
    assert_eq!(
        open.map(|open| (open.turn, open.works)),
        Some((seq(6), true))
    );
    // A turn that only talked looked at nothing: the mark stays where the last look stopped.
    assert_eq!(
        mark,
        Some(Mark {
            turn: seq(1),
            since_ns: 42
        })
    );
    let (at_home, _) = from_log(&log, true);
    assert_eq!(
        at_home.map(|open| open.works),
        Some(false),
        "a worker at home wakes it itself"
    );
    let mut looked = log.to_vec();
    looked.push(event(8, "turn.made", &json!({"turn": 1})));
    assert_eq!(from_log(&looked, false).1, None, "looked up to its end");
    let mut worked = log.get(..6).unwrap().to_vec();
    worked.push(event(7, "tool.call.requested", &call("write")));
    worked.push(event(8, "turn.ended", &json!({"outcome": "completed"})));
    assert_eq!(
        from_log(&worked, false).1,
        None,
        "a working turn looked up to its end"
    );
}

#[test]
fn an_office_lock_file_is_never_made() {
    assert!(locked("out/~$report.docx") && locked("~$a.xlsx"));
    assert!(!locked("out/report.docx") && !locked("a~$.xlsx"));
    assert!(changes_files("write", false) && changes_files("bash", true));
    assert!(!changes_files("read", false) && !changes_files("agent", true));
}
