//! `glave threads`, the real binary through the person's server: the folder's threads listed,
//! archived, brought back and deleted there, as the app does.

use glave_provider_openai::fake::{FakeServer, Reply};

use crate::http::{only_the_server_answers, serve_http};
use crate::support::*;

#[test]
fn a_folder_s_threads_are_listed_archived_and_deleted_in_the_person_s_server() {
    let place = place("threads");
    let model = FakeServer::start(vec![Reply::text("Hi.")]).unwrap();
    let first = glave(&place, &model, &["--model", "m", "hello"], "");
    assert!(first.status.success(), "{}", text(&first));
    let (_served, _) = serve_http(&place, &model, &["--model", "m"]);
    only_the_server_answers(&place);
    let threads = |words: &[&str]| {
        let output = glave(&place, &model, &[&["threads"][..], words].concat(), "");
        let said = text(&output);
        assert!(output.status.success(), "{said}");
        said
    };
    let listed = threads(&[]);
    assert_eq!(listed.lines().count(), 1, "{listed}");
    let id = listed.trim().to_owned();
    assert!(threads(&["archive", &id]).contains(&format!("archived {id}")));
    assert_eq!(threads(&[]).trim(), "");
    assert_eq!(threads(&["--archived"]).trim(), id);
    assert!(threads(&["unarchive", &id]).contains(&format!("unarchived {id}")));
    assert_eq!(threads(&[]).trim(), id);
    assert!(threads(&["delete", &id]).contains(&format!("deleted {id}")));
    assert_eq!(threads(&["--deleted"]).trim(), id);
    assert_eq!(threads(&[]).trim(), "");
}

#[test]
fn a_thread_is_forked_in_the_person_s_server_saying_its_files_are_not_rewound() {
    let place = place("threads-fork");
    let model = FakeServer::start(vec![Reply::text("Hi.")]).unwrap();
    let first = glave(&place, &model, &["--model", "m", "hello"], "");
    assert!(first.status.success(), "{}", text(&first));
    let (_served, _) = serve_http(&place, &model, &["--model", "m"]);
    only_the_server_answers(&place);
    let threads = |words: &[&str]| glave(&place, &model, &[&["threads"][..], words].concat(), "");
    let id = text(&threads(&[])).trim().to_owned();
    for at in ["1", "0"] {
        let forked = threads(&["fork", &id, "--at", at]);
        assert!(forked.status.success(), "{}", text(&forked));
        let said = String::from_utf8_lossy(&forked.stdout);
        let lines: Vec<&str> = said.lines().collect();
        assert_eq!(lines.len(), 2, "its id, then what it says: {said}");
        assert_ne!(lines[0], id);
        assert_eq!(
            lines[1],
            "Forked here from an earlier thread. Files aren't rewound: this thread works in the \
             same folder, with them as they are now."
        );
    }
    assert_eq!(
        text(&threads(&[])).lines().count(),
        3,
        "the original and two forks"
    );
    let past = threads(&["fork", &id, "--at", "2"]);
    assert!(!past.status.success());
    assert!(text(&past).contains("--at takes 0 to 1"), "{}", text(&past));
}
