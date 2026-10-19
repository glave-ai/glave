//! The file tools driven the way the model drives them, through `Tool::run`: what they do, and
//! every way they refuse.

use std::path::Path;

use glave_event::Arguments;
use glave_ports::block_on;
use glave_ports::testing::TempDir;
use serde_json::json;

use super::workspace::Workspace;
use super::{Tool, ToolOutcome};
use crate::cancel::Cancel;

fn project(name: &str) -> TempDir {
    TempDir::new(&format!("tools-{name}")).unwrap()
}

/// One workspace for a sequence of calls, as in a real thread (reads are remembered).
struct Session {
    workspace: Workspace,
}

impl Session {
    fn new(root: &Path) -> Self {
        Self {
            workspace: block_on!(Workspace::open(root, std::env::vars_os())).unwrap(),
        }
    }

    fn call(&self, tool: Tool, arguments: serde_json::Value) -> ToolOutcome {
        let arguments = Arguments::Json(serde_json::from_value(arguments).unwrap());
        block_on!(tool.run(
            &arguments,
            (&self.workspace, &crate::background::Commands::default()),
            &Cancel::new()
        ))
    }
}

#[test]
fn write_creates_files_and_folders_but_never_overwrites_unread_or_stale_files() {
    let root = project("write");
    let s = Session::new(&root);
    let created = s.call(
        Tool::Write,
        json!({"path": "a/b/new.md", "content": "one\ntwo\n"}),
    );
    assert!(!created.is_error, "{}", created.text);
    assert!(created.text.contains("new file"), "{}", created.text);
    assert_eq!(
        std::fs::read_to_string(root.join("a/b/new.md")).unwrap(),
        "one\ntwo\n"
    );

    std::fs::write(root.join("existing.md"), "old\n").unwrap();
    let unread = s.call(
        Tool::Write,
        json!({"path": "existing.md", "content": "new\n"}),
    );
    assert!(unread.is_error && unread.text.contains("Read existing.md before changing it"));
    assert_eq!(
        std::fs::read_to_string(root.join("existing.md")).unwrap(),
        "old\n"
    );

    assert!(!s.call(Tool::Read, json!({"path": "existing.md"})).is_error);
    std::fs::write(root.join("existing.md"), "changed by the person\n").unwrap();
    let stale = s.call(
        Tool::Write,
        json!({"path": "existing.md", "content": "new\n"}),
    );
    assert!(stale.is_error && stale.text.contains("changed since you last read it"));

    assert!(!s.call(Tool::Read, json!({"path": "existing.md"})).is_error);
    let replaced = s.call(
        Tool::Write,
        json!({"path": "existing.md", "content": "new\n"}),
    );
    assert!(
        replaced.text.contains("replaced the existing file"),
        "{}",
        replaced.text
    );

    let folder = s.call(Tool::Write, json!({"path": "a", "content": "x"}));
    assert!(folder.is_error && folder.text.contains("is a folder, not a file"));
    let outside = s.call(Tool::Write, json!({"path": "../escape.md", "content": "x"}));
    assert!(outside.is_error && outside.text.contains("outside the workspace"));
}

#[test]
fn edit_explains_every_refusal() {
    let root = project("edit-refusals");
    let s = Session::new(&root);
    let missing = s.call(
        Tool::Edit,
        json!({"path": "none.rs", "old_string": "a", "new_string": "b"}),
    );
    assert!(missing.is_error && missing.text.contains("To create a file, use write"));

    std::fs::write(root.join("lib.rs"), "fn a() {}\nfn a() {}\nfn b() {}\n").unwrap();
    let unread = s.call(
        Tool::Edit,
        json!({"path": "lib.rs", "old_string": "fn b", "new_string": "fn c"}),
    );
    assert!(unread.is_error && unread.text.contains("Read lib.rs before changing it"));

    assert!(!s.call(Tool::Read, json!({"path": "lib.rs"})).is_error);
    let edit = |old: &str, new: &str| {
        s.call(
            Tool::Edit,
            json!({"path": "lib.rs", "old_string": old, "new_string": new}),
        )
    };
    let ambiguous = edit("fn a() {}", "fn z() {}");
    assert!(
        ambiguous.is_error
            && ambiguous
                .text
                .contains("matches 2 places in lib.rs (lines 1, 2)")
    );
    let not_found = edit("fn q() {}", "fn z() {}");
    assert!(not_found.is_error && not_found.text.contains("was not found in lib.rs"));
    assert!(edit("", "x").text.contains("old_string is empty"));
    assert!(edit("fn b() {}", "fn b() {}").text.contains("identical"));
    assert_eq!(
        std::fs::read_to_string(root.join("lib.rs")).unwrap(),
        "fn a() {}\nfn a() {}\nfn b() {}\n",
        "no refusal changed the file"
    );

    std::fs::write(root.join("latin1.txt"), b"caf\xe9\n").unwrap();
    assert!(!s.call(Tool::Read, json!({"path": "latin1.txt"})).is_error);
    let not_utf8 = s.call(
        Tool::Edit,
        json!({"path": "latin1.txt", "old_string": "caf", "new_string": "tea"}),
    );
    assert!(not_utf8.is_error && not_utf8.text.contains("is not UTF-8 text"));
}

#[test]
fn edit_keeps_line_endings_and_byte_order_mark_and_chains_without_rereading() {
    let root = project("edit-endings");
    let s = Session::new(&root);
    std::fs::write(root.join("win.txt"), "\u{feff}one\r\ntwo\r\nthree\r\n").unwrap();
    assert!(!s.call(Tool::Read, json!({"path": "win.txt"})).is_error);
    let first = s.call(
        Tool::Edit,
        json!({"path": "win.txt", "old_string": "two", "new_string": "2"}),
    );
    assert!(
        first
            .text
            .contains("Edited win.txt: 1 replacement at line 2"),
        "{}",
        first.text
    );
    let second = s.call(
        Tool::Edit,
        json!({"path": "win.txt", "old_string": "one\nTWO", "new_string": "x"}),
    );
    assert!(
        second.is_error,
        "the file changed, so the old text is gone: {}",
        second.text
    );
    let chained = s.call(
        Tool::Edit,
        json!({"path": "win.txt", "old_string": "three", "new_string": "3", "replace_all": true}),
    );
    assert!(
        !chained.is_error,
        "glave's own edit keeps the read fresh: {}",
        chained.text
    );
    assert_eq!(
        std::fs::read(root.join("win.txt")).unwrap(),
        "\u{feff}one\r\n2\r\n3\r\n".as_bytes()
    );
}

#[test]
fn read_shows_files_and_folders_and_refuses_what_it_cannot_show() {
    let root = project("read");
    let s = Session::new(&root);
    std::fs::write(root.join(".gitignore"), "build/\n").unwrap();
    std::fs::create_dir_all(root.join("build")).unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("empty")).unwrap();
    std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(root.join("blank.txt"), "").unwrap();
    std::fs::write(root.join("logo.png"), b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR").unwrap();
    std::fs::File::create(root.join("huge.log"))
        .unwrap()
        .set_len(21 * 1024 * 1024)
        .unwrap();

    let listing = s.call(Tool::Read, json!({"path": "."}));
    assert!(listing.text.contains("src/") && listing.text.contains(".gitignore"));
    assert!(
        !listing.text.lines().any(|line| line == "build/"),
        "{}",
        listing.text
    );
    assert!(
        listing.text.contains("(1 more that git ignores"),
        "{}",
        listing.text
    );
    assert_eq!(
        s.call(Tool::Read, json!({"path": "empty"})).text,
        "(empty folder)"
    );

    assert_eq!(
        s.call(Tool::Read, json!({"path": "src/main.rs"})).text,
        "1\tfn main() {}\n(end of file, 1 lines)"
    );
    assert_eq!(
        s.call(Tool::Read, json!({"path": "blank.txt"})).text,
        "(empty file)"
    );
    let binary = s.call(Tool::Read, json!({"path": "logo.png"}));
    assert!(binary.is_error && binary.text.contains("is a binary file"));
    let huge = s.call(Tool::Read, json!({"path": "huge.log"}));
    assert!(huge.is_error && huge.text.contains("too large to read"));
    let missing = s.call(Tool::Read, json!({"path": "nope.md"}));
    assert!(missing.is_error && missing.text.contains("File not found: nope.md"));
}

#[test]
fn bad_arguments_are_explained_to_the_model() {
    let root = project("arguments");
    let s = Session::new(&root);
    let wrong_type = s.call(Tool::Write, json!({"path": 7}));
    assert!(
        wrong_type.is_error
            && wrong_type
                .text
                .contains("The arguments for write are invalid")
    );
    let malformed = block_on!(Tool::Read.run(
        &Arguments::Malformed("{\"path\": ".into()),
        (&s.workspace, &crate::background::Commands::default()),
        &Cancel::new(),
    ));
    assert!(malformed.is_error && malformed.text.contains("not valid JSON"));
}
