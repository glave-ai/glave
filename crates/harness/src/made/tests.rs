use std::io::Write as _;

use flate2::Compression;
use flate2::write::DeflateEncoder;
use glave_event::{RawEvent, decode};
use glave_ports::testing::any_secret;
use serde_json::{Value, json};

use super::scan::{Scanned, scanned};
use super::*;

/// A zip of one part, `name`, deflated, holding `content`, as an Office file keeps its XML:
/// only its local header, which is all that is read of it.
fn zipped(name: &str, content: &[u8]) -> Vec<u8> {
    let mut deflating = DeflateEncoder::new(Vec::new(), Compression::fast());
    deflating.write_all(content).unwrap();
    let data = deflating.finish().unwrap();
    let mut zip = b"PK\x03\x04".to_vec();
    for field in [20_u16, 0, 8, 0, 0, 0, 0] {
        zip.extend_from_slice(&field.to_le_bytes());
    }
    zip.extend_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
    zip.extend_from_slice(&u32::try_from(content.len()).unwrap().to_le_bytes());
    zip.extend_from_slice(&u16::try_from(name.len()).unwrap().to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(name.as_bytes());
    zip.extend_from_slice(&data);
    zip
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
fn a_part_that_inflates_past_what_is_looked_through_can_t_be_told() {
    let bomb = zipped("xl/sharedStrings.xml", &vec![0; 65 * 1024 * 1024]);
    assert!(bomb.len() < 1024 * 1024, "{}", bomb.len());
    let values = [any_secret().into_bytes()];
    assert_eq!(scanned(&bomb, &values), Scanned::Unchecked);
    assert_eq!(
        scanned(&bomb, &[]),
        Scanned::Clean,
        "with no values, nothing is inflated"
    );
}

/// The event of `kind` with `data` at `seq`, as a thread's log holds it, decoded.
fn event(seq: u64, kind: &str, data: &Value) -> Event {
    let line = json!({
        "glave": 1,
        "id": format!("0192d6a0-0000-7000-8000-{seq:012}"),
        "thread": "0192d6a0-0000-7000-8000-000000000000",
        "seq": seq,
        "at": "2026-11-16T12:00:00Z",
        "type": kind,
        "v": 1,
        "data": data,
    });
    decode(RawEvent::from_json(&line.to_string()).unwrap()).unwrap()
}

#[test]
fn where_a_thread_stands_is_read_again_from_its_log() {
    let config = json!({"sha256": "9".repeat(64), "bytes": 2, "media_type": "application/json"});
    let started = json!({"model": {"provider": "p", "name": "m"}, "config": config});
    let call = |tool: &str| json!({"call_id": "call_1", "tool": tool, "arguments": {"json": {}}});
    let waits = json!({"outcome": "completed", "made_left": {"kind": "waits"}});
    let log = [
        event(1, "turn.started", &started),
        event(2, "tool.call.requested", &call("bash")),
        event(3, "turn.ended", &waits),
        event(4, "turn.started", &started),
        event(5, "tool.call.requested", &call("agent")),
    ];
    let (open, waiting) = from_log(&log, false);
    let seq = |seq: u64| {
        log.iter()
            .find(|event| event.header.seq.get() == seq)
            .unwrap()
    };
    assert_eq!(
        open.map(|open| (open.turn, open.works)),
        Some((seq(4).header.seq, true))
    );
    assert_eq!(waiting.map(|waiting| waiting.turn), Some(seq(1).header.seq));
    let (at_home, _) = from_log(&log, true);
    assert_eq!(
        at_home.map(|open| open.works),
        Some(false),
        "a worker at home wakes it itself"
    );
    let mut looked = log.to_vec();
    looked.push(event(6, "turn.made", &json!({"turn": 1})));
    assert_eq!(from_log(&looked, false).1, None);
}

#[test]
fn an_office_lock_file_is_never_made() {
    assert!(locked("out/~$report.docx") && locked("~$a.xlsx"));
    assert!(!locked("out/report.docx") && !locked("a~$.xlsx"));
    assert!(changes_files("write", false) && changes_files("bash", true));
    assert!(!changes_files("read", false) && !changes_files("agent", true));
}
