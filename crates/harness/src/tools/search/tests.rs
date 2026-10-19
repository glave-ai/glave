use std::path::Path;

use glave_ports::block_on;
use glave_ports::testing::TempDir;

use super::*;

fn project(name: &str, files: &[(&str, &[u8])]) -> TempDir {
    let dir = TempDir::new(&format!("search-{name}")).unwrap();
    for (path, contents) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }
    dir
}

fn run(dir: &Path, args: serde_json::Value) -> ToolOutcome {
    let args: SearchArgs = serde_json::from_value(args).unwrap();
    block_on!(async {
        let workspace = Workspace::open(dir, []).await.unwrap();
        search(&workspace, args).await
    })
}

#[test]
fn matches_show_path_line_and_text_and_skip_what_must_not_be_searched() {
    let dir = project(
        "skips",
        &[
            (".gitignore", b"target/\n"),
            (
                "src/lib.rs",
                b"fn load() {}\npub fn read_settings_file() {}\n",
            ),
            ("target/copy.rs", b"pub fn read_settings_file() {}\n"),
            (".env", b"read_settings_file=secret\n"),
            ("logo.png", b"\x89PNG\0\0read_settings_file"),
        ],
    );
    let out = run(&dir, serde_json::json!({ "pattern": "fn read_\\w+" }));
    assert!(!out.is_error);
    assert_eq!(
        out.text,
        "src/lib.rs:2:pub fn read_settings_file() {}\n\
         [Not searched, since they usually hold secrets: .env]"
    );
}

#[test]
fn a_broken_regex_is_searched_as_plain_text() {
    let dir = project("text", &[("a.rs", b"call(x)\nother\n")]);
    let out = run(&dir, serde_json::json!({ "pattern": "call(" }));
    assert_eq!(
        out.text,
        "(Not a valid regular expression, so it was searched as plain text.)\na.rs:1:call(x)"
    );
}

#[test]
fn glob_path_and_case_narrow_the_search() {
    let dir = project(
        "narrow",
        &[
            ("src/a.rs", b"Needle\n"),
            ("src/b.md", b"needle\n"),
            ("docs/c.rs", b"needle\n"),
        ],
    );
    let out = run(
        &dir,
        serde_json::json!({ "pattern": "needle", "path": "src", "glob": "*.rs", "ignore_case": true }),
    );
    assert_eq!(out.text, "src/a.rs:1:Needle");
    let none = run(
        &dir,
        serde_json::json!({ "pattern": "haystack", "glob": "*.rs" }),
    );
    assert_eq!(
        none.text,
        "No matches for \"haystack\" in . (files matching *.rs)."
    );
}

#[test]
fn many_matches_stop_with_a_hint() {
    let lines = "hit\n".repeat(MAX_MATCHES + 5);
    let dir = project("many", &[("big.txt", lines.as_bytes())]);
    let out = run(&dir, serde_json::json!({ "pattern": "hit" }));
    assert_eq!(out.text.lines().count(), MAX_MATCHES + 1);
    assert!(
        out.text
            .ends_with("Narrow the pattern, path or glob to see the rest.]")
    );
}

#[test]
fn searching_outside_the_project_is_refused() {
    let dir = project("outside", &[("a.rs", b"x\n")]);
    let out = run(&dir, serde_json::json!({ "pattern": "x", "path": "../" }));
    assert!(out.is_error);
}
