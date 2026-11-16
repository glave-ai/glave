use serde_json::{Value, json};

use super::*;

/// Reads the events made of `kinds_and_data`, in order, as `thread/files` does.
fn read(kinds_and_data: &[(&str, Value)]) -> Vec<TurnFiles> {
    let mut reading = Reading::default();
    for (seq, (kind, data)) in (1_u64..).zip(kinds_and_data) {
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
        reading.read(RawEvent::from_json(&line.to_string()).unwrap());
    }
    reading.turns
}

fn started() -> (&'static str, Value) {
    let config = json!({"sha256": "9".repeat(64), "bytes": 2, "media_type": "application/json"});
    let data = json!({"model": {"provider": "p", "name": "m"}, "config": config});
    ("turn.started", data)
}

/// A call of `tool`, `id`, asked for and finished, saying it made each of `made`: a path and its
/// content's hash, a PDF.
fn call(id: &str, tool: &str, made: &[(&str, char)]) -> [(&'static str, Value); 2] {
    let made: Vec<Value> = made
        .iter()
        .map(|(path, hash)| {
            let blob = json!({"sha256": hash.to_string().repeat(64), "bytes": 9, "media_type": "application/pdf"});
            json!({"path": path, "blob": blob})
        })
        .collect();
    let requested = json!({"call_id": id, "tool": tool, "arguments": {"json": {}}});
    let completed =
        json!({"call_id": id, "is_error": false, "output": [], "structured": {"made": made}});
    [
        ("tool.call.requested", requested),
        ("tool.call.completed", completed),
    ]
}

#[test]
fn each_turn_gives_each_file_it_made_once_as_last_made() {
    let mut events = vec![started()];
    events.extend(call("a", "bash", &[("out/Trip budget.pdf", 'a')]));
    events.extend(call("b", "write", &[("notes.pdf", 'b')]));
    events.extend(call("c", "bash", &[("out/Trip budget.pdf", 'c')]));
    events.push(started());
    events.extend(call("d", "web_fetch", &[("forged.pdf", 'd')]));
    let turns = read(&events);
    let turn: Vec<(&str, &str, String, FileKind, u64)> = turns[0]
        .files
        .iter()
        .map(|file| {
            let hash = file.sha256.as_str().get(..1).unwrap_or_default().to_owned();
            (
                file.name.as_str(),
                file.title.as_str(),
                hash,
                file.kind,
                file.seq.get(),
            )
        })
        .collect();
    assert_eq!(
        turn,
        [
            (
                "Trip budget.pdf",
                "Trip budget",
                "c".to_owned(),
                FileKind::Pdf,
                7
            ),
            ("notes.pdf", "notes", "b".to_owned(), FileKind::Pdf, 5),
        ]
    );
    assert_eq!(&*turns[0].files[0].kept, "Kept with this chat");
    assert_eq!(turns[1].turn.get(), 8);
    assert!(
        turns[1].files.is_empty(),
        "only glave's own tools make files"
    );
}
