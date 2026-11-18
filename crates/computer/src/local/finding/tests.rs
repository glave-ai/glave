use std::time::{Duration, SystemTime};

use glave_ports::block_on;
use glave_ports::computer::Glob;
use glave_ports::testing::TempDir;

use super::glob;

/// Writes `path` in `dir`, changed `nanos` after 1970.
fn changed_at(dir: &std::path::Path, path: &str, nanos: u64) {
    let path = dir.join(path);
    std::fs::write(&path, "%PDF-1.7").unwrap();
    let file = std::fs::File::options().write(true).open(&path).unwrap();
    let at = SystemTime::UNIX_EPOCH + Duration::from_nanos(nanos);
    file.set_modified(at).unwrap();
}

fn pdfs() -> Glob {
    Glob {
        pattern: "*.pdf".into(),
        path: ".".into(),
        limit: 10,
        ..Glob::default()
    }
}

#[test]
fn a_look_over_a_window_names_what_changed_after_its_start_and_by_its_end() {
    let dir = TempDir::new("finding-window").unwrap();
    let at = 1_700_000_000_000_000_000;
    changed_at(dir.path(), "edge.pdf", at);
    // Changed in 2030, later than now, as a file being written may seem to be.
    changed_at(dir.path(), "ahead.pdf", 1_900_000_000_000_000_000);
    let since = |nanos| Glob {
        changed_since_ns: Some(nanos),
        ..pdfs()
    };
    let (at_start, before_start, plain) = block_on!(async {
        (
            glob(dir.path(), &since(at)).await.unwrap(),
            glob(dir.path(), &since(at - 1)).await.unwrap(),
            glob(dir.path(), &pdfs()).await.unwrap(),
        )
    });
    // A file changed at the very time a window starts was the last window's.
    assert_eq!(at_start.files, Vec::<String>::new());
    assert_eq!(before_start.files, ["edge.pdf"]);
    // A window ends now, and says so: a file ahead of the clock is the next one's.
    assert!(before_start.cut_off_ns.is_some_and(|cut_off| cut_off > at));
    // A plain look has no window: it finds both.
    assert_eq!(plain.total, 2);
    assert_eq!(plain.cut_off_ns, None);
}

#[test]
fn a_look_held_to_a_number_of_entries_finds_all_in_a_large_folder_under_it() {
    let dir = TempDir::new("finding-flat").unwrap();
    for n in 0..5_000 {
        std::fs::write(dir.path().join(format!("note-{n}.txt")), "").unwrap();
    }
    std::fs::write(dir.path().join("made.pdf"), "%PDF-1.7").unwrap();
    let held = Glob {
        most_looked_at: Some(50_000),
        ..pdfs()
    };
    let found = block_on!(glob(dir.path(), &held)).unwrap();
    assert_eq!(found.files, ["made.pdf"]);
    assert!(!found.stopped);
    let stopped = block_on!(glob(
        dir.path(),
        &Glob {
            most_looked_at: Some(100),
            ..pdfs()
        }
    ))
    .unwrap();
    assert!(
        stopped.stopped,
        "past its entries, it stops however large the folder"
    );
}
