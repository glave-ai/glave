use super::*;

fn arguments(value: serde_json::Value) -> Arguments {
    match value {
        serde_json::Value::Object(map) => Arguments::Json(map),
        other => panic!("not an object: {other}"),
    }
}

/// Each line of `lines` as its sign and text: `+ new`, `- old`, `  kept`.
fn shown(lines: &[DiffLine]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            let sign = match line.kind {
                LineKind::Same => ' ',
                LineKind::Removed => '-',
                LineKind::Added => '+',
            };
            format!("{sign} {}", line.text)
        })
        .collect()
}

#[test]
fn an_edit_keeps_the_lines_both_sides_share_at_its_ends() {
    let before = "fn parse() {\n    old()\n}\n";
    let after = "fn parse() {\n    new()\n    more()\n}\n";
    assert_eq!(
        shown(&edited(before, after)),
        [
            "  fn parse() {",
            "-     old()",
            "+     new()",
            "+     more()",
            "  }"
        ]
    );
    assert_eq!(shown(&edited("", "a")), ["+ a"], "nothing taken out");
    assert_eq!(shown(&edited("a\r\nb", "a\r\nc")), ["  a", "- b", "+ c"]);
    assert_eq!(shown(&edited("same", "same")), ["  same"]);
}

#[test]
fn a_write_puts_every_line_and_only_glave_s_own_calls_change_files() {
    let write = arguments(serde_json::json!({"path": "a.txt", "content": "one\ntwo\n"}));
    let (path, change) = asked("write", &write).unwrap();
    assert_eq!(path, "a.txt");
    assert_eq!(change.kind, ChangeKind::Wrote);
    assert_eq!(shown(&change.lines), ["+ one", "+ two"]);
    let every = arguments(serde_json::json!({
        "path": "a.txt", "old_string": "x", "new_string": "y", "replace_all": true
    }));
    assert_eq!(
        asked(" Edit", &every).unwrap().1.kind,
        ChangeKind::EditedEvery
    );
    assert_eq!(
        asked("bash", &write),
        None,
        "a command's changes are not known"
    );
    assert_eq!(asked("mcp__files__write", &write), None);
    let nowhere = arguments(serde_json::json!({"content": "x"}));
    assert_eq!(asked("write", &nowhere), None, "no file named");
    assert_eq!(asked("write", &Arguments::Malformed("{".into())), None);
}

#[test]
fn files_add_up_their_changes_in_the_order_first_changed() {
    let change = |turn: u64, path: &str, before: &str, after: &str| Made {
        turn: Seq::new(turn).ok(),
        path: path.into(),
        change: FileChange {
            kind: ChangeKind::Edited,
            lines: edited(before, after),
        },
    };
    let changes = [
        change(3, "b.rs", "x", "y\nz"),
        change(3, "a.rs", "", "a"),
        change(9, "b.rs", "y", "w"),
    ];
    let files = files_of(changes.iter());
    let counted: Vec<_> = files
        .iter()
        .map(|file| {
            (
                file.path.as_str(),
                file.added,
                file.removed,
                file.changes.len(),
            )
        })
        .collect();
    assert_eq!(counted, [("b.rs", 3, 2, 2), ("a.rs", 1, 0, 1)]);
    let turns = turns_of(&changes);
    let by_turn: Vec<_> = turns
        .iter()
        .map(|turn| (turn.turn.get(), turn.files.len()))
        .collect();
    assert_eq!(by_turn, [(3, 2), (9, 1)]);
}
