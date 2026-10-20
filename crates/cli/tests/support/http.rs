//! `glave serve --http` for the binary's end-to-end tests: the person's server, started on a
//! free port of this computer, talking only to a fake model server.

use std::io::{BufRead as _, BufReader};
use std::process::{Command, Stdio};

use glave_ports::testing::{FAKE_SECRET, Running};
use glave_provider_openai::fake::FakeServer;

use crate::support::Place;

/// `glave serve --http` for `place` with `args`, talking only to `server`, stopped when
/// dropped, and the address it serves on, once it answers on its socket too.
pub(crate) fn serve_http(place: &Place, server: &FakeServer, args: &[&str]) -> (Running, String) {
    // No titles: a title call would race the next turn for the fake model's replies. First,
    // so a table the test's own settings end with does not take it in.
    std::fs::create_dir_all(&place.home).unwrap();
    let config = place.home.join("config.toml");
    let own = std::fs::read_to_string(&config).unwrap_or_default();
    if !own.starts_with("titles = false\n") {
        std::fs::write(&config, format!("titles = false\n{own}")).unwrap();
    }
    let child = Command::new(env!("CARGO_BIN_EXE_glave"))
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
    let mut running = Running::new(child);
    let mut lines = BufReader::new(running.stderr.take().unwrap()).lines();
    let mut url = None;
    while let Some(line) = lines.next().map(Result::unwrap) {
        if let Some(address) = line.strip_prefix("glave: serving the session API on ") {
            url = Some(address.to_owned());
        }
        if line.starts_with("glave: serving you on ") {
            break;
        }
    }
    // Whatever it says later is still read, so it never blocks.
    std::thread::spawn(move || for _ in lines.map_while(Result::ok) {});
    (running, url.unwrap())
}

/// Leaves `place`'s settings unreadable, so a command run there cannot answer in its own
/// process and only the person's server, which read them as it started, answers it.
pub(crate) fn only_the_server_answers(place: &Place) {
    std::fs::write(place.home.join("config.toml"), "this is not = = toml\n").unwrap();
}
