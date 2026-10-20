use serde_json::json;

use super::*;

/// A workspace like glave's: the event types, the ports on them, a store on the ports, the
/// binary on everything, the eval crate on the ports, and xtask alone.
fn workspace() -> Vec<Package> {
    let metadata = json!({
        "workspace_root": "/w",
        "packages": [
            {"name": "glave-event", "manifest_path": "/w/crates/event/Cargo.toml",
             "dependencies": [{"name": "serde"}]},
            {"name": "glave-ports", "manifest_path": "/w/crates/ports/Cargo.toml",
             "dependencies": [{"name": "glave-event"}]},
            {"name": "glave-store", "manifest_path": "/w/crates/store/Cargo.toml",
             "dependencies": [{"name": "glave-ports"}]},
            {"name": "glave-eval", "manifest_path": "/w/crates/eval/Cargo.toml",
             "dependencies": [{"name": "glave-ports"}]},
            {"name": "glave", "manifest_path": "/w/crates/cli/Cargo.toml",
             "dependencies": [{"name": "glave-store"}, {"name": "glave-eval"}]},
            {"name": "glave-mcp", "manifest_path": "/w/crates/mcp/Cargo.toml",
             "dependencies": [{"name": "glave-ports"}]},
            {"name": "xtask", "manifest_path": "/w/xtask/Cargo.toml", "dependencies": []},
        ]
    });
    packages(&metadata).unwrap()
}

fn changed(files: &[&str]) -> Scope {
    let files: Vec<String> = files.iter().map(|file| (*file).to_owned()).collect();
    reached(&files, &workspace())
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn a_change_reaches_its_crate_and_every_crate_that_depends_on_it() {
    let Scope::Changed(change) = changed(&["crates/store/src/lib.rs"]) else {
        panic!("only what changed")
    };
    assert_eq!(change.touched, set(&["glave-store"]));
    // glave's tests run glave-mcp's test server, so it is built and checked too.
    assert_eq!(change.reached, set(&["glave-store", "glave", "glave-mcp"]));
    assert!(!change.web);
    let Scope::Changed(change) = changed(&["crates/event/src/lib.rs"]) else {
        panic!("only what changed")
    };
    assert_eq!(
        change.reached,
        set(&[
            "glave-event",
            "glave-ports",
            "glave-store",
            "glave-eval",
            "glave",
            "glave-mcp"
        ])
    );
}

#[test]
fn the_web_runs_for_its_own_files_and_the_schema() {
    let Scope::Changed(change) = changed(&["web/app/src/main.ts", "AGENTS.md"]) else {
        panic!("only what changed")
    };
    assert!(change.web && change.reached.is_empty());
    let Scope::Changed(change) = changed(&["specs/session.schema.json"]) else {
        panic!("only what changed")
    };
    assert!(change.web);
}

#[test]
fn prose_and_workflows_reach_nothing_and_the_evals_reach_their_crate() {
    assert_eq!(
        changed(&["specs/gateway.md", ".github/workflows/ci.yml"]),
        Scope::Changed(Change::default())
    );
    let Scope::Changed(change) = changed(&["evals/tasks/fix-bug/instruction.md"]) else {
        panic!("only what changed")
    };
    assert_eq!(change.reached, set(&["glave-eval", "glave", "glave-mcp"]));
}

#[test]
fn a_file_every_crate_may_read_checks_everything() {
    for file in ["Cargo.lock", "clippy.toml", ".config/nextest.toml"] {
        assert!(
            matches!(changed(&[file, "crates/event/src/lib.rs"]), Scope::All(_)),
            "{file}"
        );
    }
}

#[test]
fn a_file_belongs_to_the_crate_whose_folder_holds_it_most_closely() {
    let mut packages = workspace();
    packages.push(Package {
        name: "nested".to_owned(),
        folder: "crates/store/nested".to_owned(),
        depends_on: Vec::new(),
    });
    assert_eq!(
        owner("crates/store/nested/src/lib.rs", &packages).as_deref(),
        Some("nested")
    );
    assert_eq!(
        owner("crates/storeroom/x.rs", &packages),
        None,
        "a folder's name is not a prefix of another's"
    );
}
