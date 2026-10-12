//! The CLI as a client of a session server elsewhere: `glave serve --http` in one home, and
//! `glave --server` from another with nothing of its own, signed in with the server's token.
//! The turn runs there and shows here, `--continue` continues it there, and nothing is kept
//! on the client's side. Without a token, it says how to sign in.

#[cfg(test)]
#[path = "support/binary.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod remote {
    use std::io::{BufRead as _, BufReader};
    use std::process::{Child, Command, Stdio};

    use glave_provider_openai::fake::{FakeServer, Reply};

    use super::support::*;

    /// `glave serve --http` for `place`, and where it listens.
    fn serve(place: &Place, server: &FakeServer) -> (Child, String) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_glave"))
            .args(["serve", "--http", "127.0.0.1:0", "--model", "fake-model"])
            .current_dir(&place.project)
            .env("GLAVE_BASE_URL", server.url())
            .env("GLAVE_HOME", &place.home)
            .env("GLAVE_API_KEY", "sk-e2e-test")
            .env_remove("GLAVE_MODEL")
            .env_remove("GLAVE_PROVIDER")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
        let url = loop {
            let line = lines.next().unwrap().unwrap();
            if let Some(url) = line.strip_prefix("glave: serving the session API on ") {
                break url.to_owned();
            }
        };
        // Whatever it says later is still read, so it never blocks.
        std::thread::spawn(move || for _ in lines.map_while(Result::ok) {});
        (child, url)
    }

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
        let (mut served, url) = serve(&place, &model);
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
        let local = client(
            &place,
            &["--server", &url, "--model", "m", "hi"],
            Some(&token),
        );
        assert!(text(&local).contains("--model"), "{}", text(&local));
        let _ = served.kill();
        let _ = served.wait();
    }
}
