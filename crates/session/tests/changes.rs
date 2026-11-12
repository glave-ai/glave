//! Changes (`specs/session.md`, "Changes"): what a thread changed in files. In a repository,
//! the thread's are everything its folder says differs from where it started, whatever changed
//! it; elsewhere, and for each turn, what glave's own `write` and `edit` changed, read from its
//! log, its subagents' with its own, in the caller's language, and each turn's files as its end
//! records them.

use glave_event::{Body, ThreadId, decode};
use glave_ports::InMemoryLog;
use glave_session::protocol::{
    ChangeKind, ChangedFile, DiffLine, LineKind, Reply, Request, ThreadChanges, code,
};
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
        let history = events(&server, thread).await;
        let starts: Vec<u64> = history
            .iter()
            .filter(|raw| matches!(decode((*raw).clone()).unwrap().body, Body::TurnStarted(_)))
            .map(|raw| raw.seq.get())
            .collect();
        let recorded: Vec<Vec<String>> = history
            .into_iter()
            .filter_map(|raw| match decode(raw).unwrap().body {
                Body::TurnEnded(ended) => Some(ended.changed),
                _ => None,
            })
            .collect();
        server.shutdown().await;
        let Ok(Reply::Changes { changes }) = changed else {
            panic!("changes, not {changed:?}")
        };
        let ThreadChanges { files, turns, note } = changes;
        assert_eq!(
            &*note,
            "This folder keeps no version history, so only glave's own edits are listed, not \
             what commands or other tools changed.",
            "a folder in no repository, said plainly"
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
        let told: Vec<Vec<&str>> = by_turn.iter().map(|(_, paths)| paths.clone()).collect();
        assert_eq!(
            recorded, told,
            "each turn's end records the files it changed, as the changes give them"
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

/// In a repository: glave writes a file a formatter then rewrites, commands edit a tracked
/// file, make a new one and have the repository ignore another, which glave then writes.
fn script_in_repository() -> Vec<glave_ports::Completion> {
    let write = json!({"path": "main.rs", "content": "fn  main()  {}\n"});
    let format = "tr -s ' ' < main.rs > main.rs.new && mv main.rs.new main.rs";
    let edit = "printf 'Use spaces.\\n' > AGENTS.md && printf 'made\\n' > made.txt && \
        printf 'skip.txt\\n' > .gitignore";
    let ignored = json!({"path": "./skip.txt", "content": "kept\n"});
    vec![
        answer(None, Some(("write", write))),
        answer(None, Some(("bash", json!({"command": format})))),
        answer(None, Some(("bash", json!({"command": edit})))),
        answer(None, Some(("write", ignored))),
        answer(Some("Done."), None),
    ]
}

/// Each change of `file` as its kind and its lines, each after its sign.
fn signed(file: &ChangedFile) -> Vec<(ChangeKind, Vec<String>)> {
    let line = |line: &DiffLine| {
        let sign = match line.kind {
            LineKind::Same => ' ',
            LineKind::Removed => '-',
            LineKind::Added => '+',
        };
        format!("{sign}{}", line.text)
    };
    let changes = file.changes.iter();
    changes
        .map(|change| (change.kind, change.lines.iter().map(line).collect()))
        .collect()
}

#[test]
fn in_a_repository_a_thread_s_files_are_all_that_differ_from_where_it_started() {
    block_on!(async {
        let mut setup = Setup::new(script_in_repository());
        setup.settings.titles = false;
        let (server, _seen, _repo, _dir) = setup
            .build_in_repository("changes-repository", InMemoryLog::new())
            .await;
        let thread = start(&server, false).await;
        turn(&server, thread, "write main and tidy up")
            .await
            .unwrap();
        let changed = server.request(Request::ThreadChanges { thread }).await;
        server.shutdown().await;
        let Ok(Reply::Changes { changes }) = changed else {
            panic!("changes, not {changed:?}")
        };
        let ThreadChanges { files, turns, note } = changes;
        let shown: Vec<_> = files
            .iter()
            .map(|file| (file.path.as_str(), signed(file)))
            .collect();
        let edited = |lines: &[&str]| lines.iter().map(|line| (*line).to_owned()).collect();
        assert_eq!(
            shown,
            [
                (
                    ".gitignore",
                    vec![(ChangeKind::Wrote, edited(&["+skip.txt"]))]
                ),
                (
                    "AGENTS.md",
                    vec![(ChangeKind::Edited, edited(&["-Use tabs.", "+Use spaces."]))]
                ),
                ("made.txt", vec![(ChangeKind::Wrote, edited(&["+made"]))]),
                (
                    "main.rs",
                    vec![(ChangeKind::Wrote, edited(&["+fn main() {}"]))]
                ),
                ("./skip.txt", vec![(ChangeKind::Wrote, edited(&["+kept"]))]),
            ],
            "a command's edit, new files no one added, what the formatter left, then glave's \
             own edit of a file the repository ignores"
        );
        assert_eq!(
            &*note,
            "The chat's files are glave's own edits and all else that differs from the version \
             it started from, whatever changed it; a turn's are only glave's own edits."
        );
        let own: Vec<_> = turns
            .iter()
            .flat_map(|turn| turn.files.iter().map(signed))
            .collect();
        assert_eq!(
            own,
            [
                vec![(ChangeKind::Wrote, edited(&["+fn  main()  {}"]))],
                vec![(ChangeKind::Wrote, edited(&["+kept"]))]
            ],
            "the turn's own writes, as they asked"
        );
    });
}
