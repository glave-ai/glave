//! A glave handed a lifeline (`GLAVE_LIFELINE`) ends when what started it ends, however that
//! ends (#558): with its starter killed outright, a `glave serve` it started and the person's
//! server a glave at a terminal started in the background both end at once.

use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use glave_ports::testing::{HUNG, Running};
use glave_provider_openai::fake::{FakeServer, Reply};

use crate::http::serve_http_with;
use crate::support::*;

/// What starts glave: a process listening on a socket of its own, as a test runner does, and
/// the socket, for `folder`.
fn starter(folder: &Path) -> (Running, Removed) {
    let socket = Removed(socket_path(folder, "starter"));
    let listen = "import socket, sys, time\n\
        held = socket.socket(socket.AF_UNIX)\n\
        held.bind(sys.argv[1])\n\
        held.listen(16)\n\
        print('listening', flush=True)\n\
        time.sleep(600)\n";
    let child = Command::new("python3")
        .args(["-c", listen])
        .arg(&socket.0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut holder = Running::new(child);
    let mut said = String::new();
    BufReader::new(holder.stdout.take().unwrap())
        .read_line(&mut said)
        .unwrap();
    assert_eq!(said, "listening\n");
    (holder, socket)
}

/// A file removed when this drops: a socket its listener, killed, cannot remove.
struct Removed(PathBuf);

impl Drop for Removed {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// The processes that hold `lock`: the person's server holds its home's.
fn holders(lock: &Path) -> Vec<u32> {
    let listed = Command::new("lsof").arg("-t").arg(lock).output().unwrap();
    String::from_utf8_lossy(&listed.stdout)
        .split_whitespace()
        .map(|pid| pid.parse().unwrap())
        .collect()
}

#[test]
fn a_glave_and_the_person_s_server_it_starts_end_with_what_started_them() {
    let served = place("tied");
    let person = place("tied-person");
    let _stopped = Stopped(&person);
    choose(&person, "model", "\"m\"");
    choose(&person, "titles", "false");
    let (mut holder, socket) = starter(served.project.parent().unwrap());
    let tied = [(Lifeline::VARIABLE, socket.0.to_str().unwrap())];
    let model = FakeServer::start(vec![Reply::text("Hello.")]).unwrap();
    let (mut serving, _url, _said) = serve_http_with(&served, &model, &[], &tied);
    let mut asked = terminal_with(&person, &model, &["say hello"], &tied);
    typed(&mut asked, "y\n");
    let said = asked.wait_with_output().unwrap();
    assert!(said.status.success(), "{}", text(&said));
    let held = holders(&person.home.join("run/server.lock"));
    assert!(!held.is_empty(), "the person's server runs");

    holder.kill().unwrap();
    holder.wait().unwrap();
    let ended = until(HUNG, || {
        serving.try_wait().unwrap().is_some() && !held.iter().any(|pid| alive(*pid))
    });
    assert!(ended, "both glaves end with what started them");
}
