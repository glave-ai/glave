use std::collections::BTreeMap;
use std::path::PathBuf;

use glave_config::trust::Decision;
use glave_ports::block_on;

use super::*;

fn home(name: &str) -> PathBuf {
    let home =
        std::env::temp_dir().join(format!("glave-trust-ledger-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    home
}

fn entry(path: &str, trusted: bool) -> Entry {
    Entry {
        path: path.to_owned(),
        remote: None,
        items: BTreeMap::from([(
            "mcp:docs".to_owned(),
            Decision {
                definition: r#"{"command":"docs-mcp"}"#.to_owned(),
                trusted,
            },
        )]),
        by: None,
    }
}

/// `entries` as `trust.toml` held them.
fn earlier_file(entries: &[(&str, bool)]) -> String {
    let mut text = "version = 1\n".to_owned();
    for (path, trusted) in entries {
        text.push_str(&format!(
            "\n[[project]]\npath = {path:?}\n\n[project.items.\"mcp:docs\"]\n\
             definition = '{{\"command\":\"docs-mcp\"}}'\ntrusted = {trusted}\n"
        ));
    }
    text
}

/// What the record says of each project: its path and whether its server is trusted.
async fn decided(trust: &Trust) -> Vec<(String, bool)> {
    let record = trust.record().await.unwrap();
    record
        .entries()
        .iter()
        .map(|entry| (entry.path.clone(), entry.items["mcp:docs"].trusted))
        .collect()
}

#[test]
fn a_decision_is_kept_in_the_store_and_the_latest_about_a_project_holds() {
    let home = home("kept");
    block_on!(async {
        let (store, trust) = Trust::in_memory(&home).await.unwrap();
        trust.keep(entry("/code/app", true)).await.unwrap();
        trust.keep(entry("/code/web", true)).await.unwrap();
        trust.keep(entry("/code/app", false)).await.unwrap();
        // Read through another handle on the same ledger, as another server would.
        let again = Trust::open(&store, trust.person, &home).await.unwrap();
        assert_eq!(
            decided(&again).await,
            [
                ("/code/web".to_owned(), true),
                ("/code/app".to_owned(), false)
            ]
        );
        let kept = again.record().await.unwrap();
        assert_eq!(
            kept.entries()[0].by,
            Some(Principal::person(trust.person)),
            "who decided is recorded"
        );
    });
    assert!(!home.join(FILE).exists(), "no file is written");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn an_earlier_trust_file_moves_into_the_store_once_and_is_never_read_again() {
    let home = home("moved");
    let file = home.join(FILE);
    std::fs::write(
        &file,
        earlier_file(&[("/code/app", true), ("/code/web", false)]),
    )
    .unwrap();
    block_on!(async {
        let (store, trust) = Trust::in_memory(&home).await.unwrap();
        assert!(!file.exists(), "the file goes once the store holds it");
        let moved = vec![
            ("/code/app".to_owned(), true),
            ("/code/web".to_owned(), false),
        ];
        assert_eq!(decided(&trust).await, moved);
        let again = Trust::open(&store, trust.person, &home).await.unwrap();
        assert_eq!(decided(&again).await, moved);
    });
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_move_stopped_before_the_file_was_deleted_changes_nothing_the_next_time() {
    let home = home("stopped");
    let file = home.join(FILE);
    let text = earlier_file(&[("/code/app", true)]);
    std::fs::write(&file, &text).unwrap();
    block_on!(async {
        let (store, trust) = Trust::in_memory(&home).await.unwrap();
        // The person decided again since; then the same file turns up, as if glave had
        // stopped after moving it in and before deleting it.
        trust.keep(entry("/code/app", false)).await.unwrap();
        std::fs::write(&file, &text).unwrap();
        let again = Trust::open(&store, trust.person, &home).await.unwrap();
        assert_eq!(decided(&again).await, [("/code/app".to_owned(), false)]);
        assert!(!file.exists());
        let (_, next) = again.read().await.unwrap();
        assert_eq!(next, 3, "nothing was moved in twice");
    });
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_damaged_earlier_file_stops_glave_naming_it_and_is_left_in_place() {
    let home = home("damaged");
    let file = home.join(FILE);
    std::fs::write(&file, "version = 2\n").unwrap();
    block_on!(async {
        let opened = Trust::in_memory(&home).await;
        let error = format!("{:#}", opened.err().unwrap());
        assert!(
            error.contains("trust.toml") && error.contains("version = 2"),
            "{error}"
        );
    });
    assert!(file.exists());
    let _ = std::fs::remove_dir_all(&home);
}
