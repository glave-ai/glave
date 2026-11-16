use serde_json::{Value, json};

use super::*;

/// The event of `kind` with `data` at `seq`, as a thread's log holds it.
fn raw(seq: u64, kind: &str, data: &Value) -> RawEvent {
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
    RawEvent::from_json(&line.to_string()).unwrap()
}

#[test]
fn each_turn_s_files_are_what_its_end_says_it_made() {
    let config = json!({"sha256": "9".repeat(64), "bytes": 2, "media_type": "application/json"});
    let started = json!({"model": {"provider": "p", "name": "m"}, "config": config});
    let blob = json!({"sha256": "a".repeat(64), "bytes": 9, "media_type": "application/pdf"});
    let made = json!([
        {"path": "out/plan.pdf", "bytes": 9, "kept": {"blob": blob}},
        {"path": "big.pdf", "bytes": 20_000_000, "kept": "too_large"},
    ]);
    let mut turns = Vec::new();
    read(&mut turns, raw(1, "turn.started", &started));
    read(
        &mut turns,
        raw(
            2,
            "turn.ended",
            &json!({"outcome": "completed", "made": made}),
        ),
    );
    read(&mut turns, raw(3, "turn.started", &started));
    read(
        &mut turns,
        raw(4, "turn.ended", &json!({"outcome": "completed"})),
    );
    let told: Vec<(u64, Vec<&str>)> = turns
        .iter()
        .map(|(seq, files)| {
            (
                seq.get(),
                files.iter().map(|file| file.path.as_str()).collect(),
            )
        })
        .collect();
    assert_eq!(told, [(1, vec!["out/plan.pdf", "big.pdf"]), (3, vec![])]);
}

#[test]
fn a_file_is_named_by_its_name_without_its_end() {
    assert_eq!(stem("Trip budget.xlsx"), "Trip budget");
    assert_eq!(stem("notes"), "notes");
    assert_eq!(kind_word(AssetKind::Deck), "deck");
}
