//! `glave threads`, the real binary through the person's server: the folder's threads listed,
//! archived, brought back and deleted there, as the app does.

#[cfg(test)]
#[path = "support/binary.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
#[path = "support/http.rs"]
mod http;

#[cfg(test)]
mod glave_binary {
    use glave_provider_openai::fake::{FakeServer, Reply};

    use super::http::{only_the_server_answers, serve_http};
    use super::support::*;

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
}
