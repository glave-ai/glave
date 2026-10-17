//! The CLI as a client of a session server elsewhere: `glave serve --http` in one home, and
//! `glave --server` from another with nothing of its own, signed in with the server's token.
//! The turn runs there and shows here, `--continue` continues it there, and nothing is kept
//! on the client's side. Without a token, it says how to sign in; a model it names goes to
//! the server, which refuses one it does not run.

#[cfg(test)]
#[path = "support/binary.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
#[path = "support/http.rs"]
mod http;

#[cfg(test)]
mod remote {
    use std::process::{Command, Stdio};

    use glave_provider_openai::fake::{FakeServer, Reply};

    use super::http::serve_http;
    use super::support::*;

    /// `glave` in a home of its own with `args`, as a client of the server at `url`.
    fn client(place: &Place, args: &[&str], token: Option<&str>) -> std::process::Output {
        let home = place.project.join("../client-home");
        let mut command = Command::new(env!("CARGO_BIN_EXE_glave"));
        command
            .args(args)
            .current_dir(&place.project)
            .env("GLAVE_HOME", &home)
            .env("NO_COLOR", "1")
            .env_remove("GLAVE_BASE_URL")
            .env_remove("GLAVE_API_KEY")
            .env_remove("GLAVE_MODEL")
            .stdin(Stdio::null());
        match token {
            Some(token) => command.env("GLAVE_SERVER_TOKEN", token),
            None => command.env_remove("GLAVE_SERVER_TOKEN"),
        };
        command.output().unwrap()
    }

    #[test]
    fn a_turn_runs_on_the_server_and_shows_here() {
        let place = place("remote");
        let model = FakeServer::start(vec![
            Reply::text("Answered over there."),
            Reply::text("Still there."),
        ])
        .unwrap();
        let (mut served, url) = serve_http(&place, &model, &["--model", "fake-model"]);
        let token = std::fs::read_to_string(place.home.join("serve.token")).unwrap();

        let first = client(&place, &["--server", &url, "where are you?"], Some(&token));
        let shown = text(&first);
        assert!(first.status.success(), "{shown}");
        assert!(shown.contains("Answered over there."), "{shown}");
        assert!(shown.contains("done"), "{shown}");
        let again = client(
            &place,
            &["--server", &url, "--continue", "and now?"],
            Some(&token),
        );
        let shown = text(&again);
        assert!(again.status.success(), "{shown}");
        assert!(shown.contains("Still there."), "{shown}");
        let received = model.received();
        assert_eq!(received.len(), 2);
        let second = said(&received[1].body, "user");
        assert!(
            second.iter().any(|text| text.contains("where are you?")),
            "the second turn continues the first thread: {second:?}"
        );
        let client_home = place.project.join("../client-home");
        assert!(!client_home.join("glave.db").exists(), "nothing kept here");

        let unsigned = client(&place, &["--server", &url, "hi"], None);
        assert!(!unsigned.status.success());
        assert!(
            text(&unsigned).contains("GLAVE_SERVER_TOKEN"),
            "{}",
            text(&unsigned)
        );
        // A model goes to the server, which refuses one it does not run.
        let unknown = client(
            &place,
            &["--server", &url, "--model", "m", "hi"],
            Some(&token),
        );
        assert!(!unknown.status.success());
        assert!(text(&unknown).contains("model/list"), "{}", text(&unknown));
        let _ = served.kill();
        let _ = served.wait();
    }
}
