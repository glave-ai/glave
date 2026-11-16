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

/// What `turns` name, by turn: each file's path, and whether something is left out.
fn told(turns: &[Turn]) -> Vec<(u64, Vec<&str>, Option<MadeLeft>)> {
    turns
        .iter()
        .map(|turn| {
            let paths = turn.files.iter().map(|file| file.path.as_str()).collect();
            (turn.turn.get(), paths, turn.left)
        })
        .collect()
}

#[test]
fn each_turn_s_files_are_what_its_end_and_a_later_look_over_say_it_made() {
    let config = json!({"sha256": "9".repeat(64), "bytes": 2, "media_type": "application/json"});
    let started = json!({"model": {"provider": "p", "name": "m"}, "config": config});
    let blob = json!({"sha256": "a".repeat(64), "bytes": 9, "media_type": "application/pdf"});
    let made = json!([
        {"path": "out/plan.pdf", "bytes": 9, "kept": {"blob": blob}},
        {"path": "big.pdf", "bytes": 20_000_000, "kept": "too_large"},
    ]);
    let waits = json!({"outcome": "completed", "made_left": {"kind": "waits"}});
    let mut turns = Vec::new();
    for event in [
        raw(1, "turn.started", &started),
        raw(
            2,
            "turn.ended",
            &json!({"outcome": "completed", "made": made}),
        ),
        raw(3, "turn.started", &started),
        raw(4, "turn.ended", &waits),
        raw(5, "turn.started", &started),
        raw(6, "turn.ended", &waits),
    ] {
        read(&mut turns, event);
    }
    let waiting = Some(MadeLeft::Waits);
    assert_eq!(
        told(&turns),
        [
            (1, vec!["out/plan.pdf", "big.pdf"], None),
            (3, vec![], waiting),
            (5, vec![], waiting)
        ]
    );
    let later = json!({"turn": 3, "made": [{"path": "log.pdf", "bytes": 1, "kept": "secret"}]});
    read(&mut turns, raw(7, "turn.made", &later));
    let told = told(&turns);
    assert_eq!(
        told.get(1..),
        Some(&[(3, vec!["log.pdf"], None), (5, vec![], None)][..])
    );
}

#[test]
fn a_file_is_named_by_its_title_else_its_cleaned_file_name() {
    assert_eq!(cleaned("Trip_budget  2026.xlsx"), "Trip budget 2026");
    assert_eq!(cleaned("notes"), "notes");
    assert_eq!(cleaned("_.pdf"), "_.pdf");
    assert_eq!(kind_word(AssetKind::Deck), "deck");
    let blob = json!({"sha256": "a".repeat(64), "bytes": 9, "media_type": "application/pdf"});
    let unread: FileMade = serde_json::from_value(
        json!({"path": "out/Lisbon_plan.pdf", "bytes": 9, "kept": {"blob": blob}}),
    )
    .unwrap();
    let shown = made_file(unread.clone());
    assert_eq!(shown.name, "Lisbon plan");
    assert!(shown.unread.is_some() && !shown.previews);
    let read = FileMade {
        read: Some(glave_event::FileRead {
            title: Some("Lisbon".into()),
            parts: Some(Parts::Pages(2)),
            preview: None,
        }),
        ..unread
    };
    let shown = made_file(read);
    assert_eq!(shown.name, "Lisbon");
    assert!(shown.unread.is_none() && shown.previews && shown.parts.is_some());
}
