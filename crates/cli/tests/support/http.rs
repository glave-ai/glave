//! `glave serve --http` for the binary's end-to-end tests: started on a free port of this
//! computer, talking only to a fake model server.

use std::io::{BufRead as _, BufReader};
use std::process::{Child, Command, Stdio};

use glave_ports::testing::FAKE_SECRET;
use glave_provider_openai::fake::FakeServer;

use crate::support::Place;

/// `glave serve --http` for `place` with `args`, talking only to `server`, and the address it
/// serves on.
pub fn serve_http(place: &Place, server: &FakeServer, args: &[&str]) -> (Child, String) {
    // No titles: a title call would race the next turn for the fake model's replies.
    std::fs::create_dir_all(&place.home).unwrap();
    std::fs::write(place.home.join("config.toml"), "titles = false\n").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_glave"))
        .args(["serve", "--http", "127.0.0.1:0"])
        .args(args)
        .current_dir(&place.project)
        .env("GLAVE_BASE_URL", server.url())
        .env("GLAVE_HOME", &place.home)
        .env("GLAVE_API_KEY", FAKE_SECRET)
        .env_remove("GLAVE_MODEL")
        .env_remove("GLAVE_PROVIDER")
        .env_remove("GLAVE_JUDGE_MODEL")
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
