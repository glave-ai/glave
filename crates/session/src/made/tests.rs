use serde_json::{Value, json};

use super::*;

/// The event of `kind` with `data` at `seq`, as a thread's log holds it.
fn raw(seq: u64, kind: &str, data: &Value) -> RawEvent {
    glave_ports::testing::logged(seq, kind, data).unwrap()
}

/// What `turns` name, by turn: each file's path, and whether something is left out.
fn told(turns: &[Turn]) -> Vec<(u64, Vec<&str>, Option<MadeLeft>)> {
    turns
        .iter()
        .map(|turn| {
            let paths = turn.files.iter().map(|file| file.path.as_str()).collect();
            (turn.started.get(), paths, turn.left)
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
    let marked = json!({"outcome": "completed", "made_until_ns": 1_794_733_198_000_000_000_u64});
    let mut turns = Vec::new();
    for event in [
        raw(1, "turn.started", &started),
        raw(
            2,
            "turn.ended",
            &json!({"outcome": "completed", "made": made}),
        ),
        raw(3, "turn.started", &started),
        raw(4, "turn.ended", &marked),
        raw(5, "turn.started", &started),
        raw(6, "turn.ended", &json!({"outcome": "completed"})),
    ] {
        read(&mut turns, event);
    }
    assert_eq!(
        told(&turns),
        [
            (1, vec!["out/plan.pdf", "big.pdf"], None),
            (3, vec![], None),
            (5, vec![], None)
        ]
    );
    // What the look-over the mark left found later is the marked turn's.
    let later = json!({"turn": 3, "made": [{"path": "log.pdf", "bytes": 1, "kept": "secret"}],
        "made_left": {"kind": "more", "files": 1}});
    read(&mut turns, raw(7, "turn.made", &later));
    let more = Some(MadeLeft::More { files: 1 });
    assert_eq!(
        told(&turns).get(1..),
        Some(&[(3, vec!["log.pdf"], more), (5, vec![], None)][..])
    );
    // A file a later look-over found again is named once, in its first place, as it is now.
    let again =
        json!({"turn": 1, "made": [{"path": "out/plan.pdf", "bytes": 12, "kept": "secret"}]});
    read(&mut turns, raw(8, "turn.made", &again));
    assert_eq!(
        told(&turns).first(),
        Some(&(1, vec!["out/plan.pdf", "big.pdf"], None))
    );
    let plan = turns.first().and_then(|turn| turn.files.first());
    assert_eq!(
        plan.map(|file| (file.bytes, &file.kept)),
        Some((12, &Kept::Secret))
    );
}

#[test]
fn a_file_is_named_by_its_cleaned_file_name() {
    assert_eq!(cleaned("Trip_budget  2026.xlsx"), "Trip budget 2026");
    assert_eq!(cleaned("notes"), "notes");
    assert_eq!(cleaned("_.pdf"), "_.pdf");
    assert_eq!(kind_word(AssetKind::Deck), "deck");
    let blob = json!({"sha256": "a".repeat(64), "bytes": 9, "media_type": "application/pdf"});
    let kept: FileMade = serde_json::from_value(
        json!({"path": "out/Lisbon_plan.pdf", "bytes": 9, "kept": {"blob": blob}}),
    )
    .unwrap();
    let shown = made_file(&kept);
    assert_eq!(shown.name, "Lisbon plan");
    assert_eq!(shown.file_name, "Lisbon_plan.pdf");
    assert!(shown.sha256.is_some());
}
