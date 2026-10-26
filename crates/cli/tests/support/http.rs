//! `glave serve --http` for the binary's end-to-end tests: the person's server, started on a
//! free port of this computer, talking only to a fake model server.

use std::io::{BufRead as _, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, channel};

use glave_ports::testing::{FAKE_SECRET, Running};
use glave_provider_openai::fake::FakeServer;

use crate::support::{Lifeline, Place, SEARCH_STAND_IN};

/// `glave serve --http` for `place` with `args`, talking only to `server`, stopped when
/// dropped, and the address it serves on, once it answers on its socket too.
pub(crate) fn serve_http(place: &Place, server: &FakeServer, args: &[&str]) -> (Running, String) {
    let (running, url, _) = serve_http_with(place, server, args, &[]);
    (running, url)
}

/// What a server said: as it started, and each line after.
pub(crate) struct Said {
    pub start: Vec<String>,
    pub later: Receiver<String>,
}

/// [`serve_http`] with the variables `env`, and what it says.
pub(crate) fn serve_http_with(
    place: &Place,
    server: &FakeServer,
    args: &[&str],
    env: &[(&str, &str)],
) -> (Running, String, Said) {
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
        .env(Lifeline::VARIABLE, &place.lifeline.path)
        .env(SEARCH_STAND_IN.0, SEARCH_STAND_IN.1)
        .envs(env.iter().copied())
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
    let mut start = Vec::new();
    while let Some(line) = lines.next().map(Result::unwrap) {
        if let Some(address) = line.strip_prefix("glave: serving the session API on ") {
            url = Some(address.to_owned());
        }
        let started = line.starts_with("glave: serving you on ");
        start.push(line);
        if started {
            break;
        }
    }
    // Whatever it says later is still read, so it never blocks, and handed on to whoever
    // listens.
    let (tell, later) = channel();
    std::thread::spawn(move || {
        for line in lines.map_while(Result::ok) {
            let _ = tell.send(line);
        }
    });
    (running, url.unwrap(), Said { start, later })
}

/// Leaves `place`'s settings unreadable, so a command run there cannot answer in its own
/// process and only the person's server, which read them as it started, answers it.
pub(crate) fn only_the_server_answers(place: &Place) {
    std::fs::write(place.home.join("config.toml"), "this is not = = toml\n").unwrap();
}
