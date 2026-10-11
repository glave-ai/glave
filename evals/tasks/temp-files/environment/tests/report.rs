use std::path::PathBuf;

use report::{load, save};

/// A fresh folder of the test's own in the temp folder.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("report-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_saved_report_loads_back() {
    let dir = scratch("round-trip");
    let path = save(&dir, &["alpha", "beta"]).unwrap();
    assert_eq!(load(&path).unwrap(), ["alpha", "beta"]);
}

#[test]
fn every_line_ends_in_a_newline() {
    let dir = scratch("newlines");
    let path = save(&dir, &["one", "two", "three"]).unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), "one\ntwo\nthree\n");
}

#[test]
fn an_empty_report_is_an_empty_file() {
    let dir = scratch("empty");
    let path = save(&dir, &[]).unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), "");
}
