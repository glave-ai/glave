//! Changes (`specs/session.md`, "Changes"): what a thread changed in files with glave's own
//! `write` and `edit`, read from its log, its subagents' with its own, for the thread and for
//! each turn, in the caller's language.

use glave_event::{Body, ThreadId, decode};
use glave_session::protocol::{ChangeKind, LineKind, Reply, Request, ThreadChanges, code};
use serde_json::json;

use crate::support::*;

/// Two turns: the first edits a file (once too soon, before reading it, which fails) and runs
/// a command that makes one; the second has a subagent write another.
fn script() -> Vec<glave_ports::Completion> {
    let edit = json!({
        "path": "main.rs",
        "old_string": "fn main() {\n    hi();",
        "new_string": "fn main() {\n    hello();\n    bye();",
    });
    let agent = json!({"description": "write notes", "prompt": "Write notes.md."});
    let notes = json!({"path": "notes.md", "content": "a\nb\n"});
    vec![
        answer(None, Some(("edit", edit.clone()))),
        answer(None, Some(("read", json!({"path": "main.rs"})))),
        answer(None, Some(("edit", edit))),
        answer(None, Some(("bash", json!({"command": "touch made.txt"})))),
        answer(Some("Edited."), None),
        answer(None, Some(("agent", agent))),
        answer(None, Some(("write", notes))),
        answer(Some("Wrote notes.md."), None),
        answer(Some("Done."), None),
    ]
}

#[test]
fn a_thread_s_changes_are_its_writes_and_edits_and_its_subagents_by_turn() {
    let dir = folder("changes");
    std::fs::write(dir.join("main.rs"), "fn main() {\n    hi();\n}\n").unwrap();
    block_on!(async {
        let mut setup = Setup::new(script());
        setup.settings.titles = false;
        setup.settings.subagents = true;
        let (server, _seen) = setup.build(&dir).await;
        let thread = start(&server, false).await;
        turn(&server, thread, "say hello").await.unwrap();
        turn(&server, thread, "and take notes").await.unwrap();
        let changed = server.request(Request::ThreadChanges { thread }).await;
        let starts: Vec<u64> = events(&server, thread)
            .await
            .into_iter()
            .filter(|raw| matches!(decode(raw.clone()).unwrap().body, Body::TurnStarted(_)))
            .map(|raw| raw.seq.get())
            .collect();
        server.shutdown().await;
        let Ok(Reply::Changes { changes }) = changed else {
            panic!("changes, not {changed:?}")
        };
        let ThreadChanges { files, turns, note } = changes;
        assert_eq!(
            &*note,
            "Changes made by commands or other tools are not listed."
        );
        let counted: Vec<_> = files
            .iter()
            .map(|file| (file.path.as_str(), file.added, file.removed))
            .collect();
        assert_eq!(
            counted,
            [("main.rs", 2, 1), ("notes.md", 2, 0)],
            "the edit that landed, and the subagent's write; never the command's file"
        );
        let main = &files[0].changes;
        assert_eq!(main.len(), 1, "the edit that failed is no change");
        assert_eq!(main[0].kind, ChangeKind::Edited);
        let kinds: Vec<_> = main[0].lines.iter().map(|line| line.kind).collect();
        assert_eq!(
            kinds,
            [
                LineKind::Same,
                LineKind::Removed,
                LineKind::Added,
                LineKind::Added
            ],
            "what the call asked for: its pieces' shared first line kept"
        );
        assert_eq!(files[1].changes[0].kind, ChangeKind::Wrote);
        let by_turn: Vec<_> = turns
            .iter()
            .map(|turn| {
                let paths: Vec<_> = turn.files.iter().map(|file| file.path.as_str()).collect();
                (turn.turn.get(), paths)
            })
            .collect();
        assert_eq!(
            by_turn,
            [(starts[0], vec!["main.rs"]), (starts[1], vec!["notes.md"])],
            "each turn by its start, the subagent's in the turn that started it"
        );
    });
}

#[test]
fn a_thread_that_changed_nothing_says_so_and_a_missing_one_is_refused() {
    let dir = folder("changes-none");
    block_on!(async {
        let (server, _seen) = Setup::new(vec![answer(Some("Hi."), None)])
            .build(&dir)
            .await;
        let thread = start(&server, false).await;
        turn(&server, thread, "hi").await.unwrap();
        let changed = server.request(Request::ThreadChanges { thread }).await;
        let missing = server
            .request(Request::ThreadChanges {
                thread: ThreadId::new(jiff::Timestamp::UNIX_EPOCH, [7; 10]).unwrap(),
            })
            .await;
        server.shutdown().await;
        let Ok(Reply::Changes { changes }) = changed else {
            panic!("changes, not {changed:?}")
        };
        assert!(changes.files.is_empty() && changes.turns.is_empty());
        assert_eq!(missing.unwrap_err().code, code::NO_THREAD);
    });
}
