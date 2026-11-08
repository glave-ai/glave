//! Where a thread's commands in the background listen, through the API: `command/list` gives
//! a command that serves on loopback its port, for the person on this computer, and none where a
//! browser of the caller's would be elsewhere, at a public origin.

use std::path::Path;

use glave_ports::host::PublicOrigin;
use glave_session::protocol::Request;
use serde_json::json;

use crate::support::*;

/// A page and a server for it, on loopback at a port the system picks, as a dev server is,
/// which writes its port (`port`) once it listens. A `TCPServer`, since `HTTPServer` asks DNS
/// for its host's name as it binds, which took over 20 seconds on macOS runners.
const SERVES: &str = "printf 'hello' > index.html && exec python3 -c \"import http.server as h, os, \
     socketserver; s = socketserver.ThreadingTCPServer(('127.0.0.1', 0), h.SimpleHTTPRequestHandler); \
     open('port.next', 'w').write(str(s.server_address[1])); os.rename('port.next', 'port'); \
     s.serve_forever()\"";

/// Waits, within its own time limit, until the server listens.
const LISTENS: &str = "while [ ! -s port ]; do sleep 0.05; done";

/// The model starts the server, then waits for it to listen, so the turn ends once it does.
fn serves() -> Vec<glave_ports::Completion> {
    vec![
        answer(
            None,
            Some(("bash", json!({"command": SERVES, "background": true}))),
        ),
        answer(
            None,
            Some(("bash", json!({"command": LISTENS, "timeout_secs": 20}))),
        ),
        answer(Some("It is serving."), None),
    ]
}

/// The port the server in `dir` said it listens on.
fn said(dir: &Path) -> u16 {
    std::fs::read_to_string(dir.join("port"))
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn a_command_that_serves_is_listed_with_its_port_for_the_person_on_this_computer() {
    let dir = folder("ports-here");
    block_on!(async {
        let (server, _) = Setup::new(serves()).build(&dir).await;
        let thread = start(&server, false).await;
        turn(&server, thread, "serve the page").await.unwrap();
        let commands = listed(&server, thread).await;
        assert_eq!(commands.len(), 1, "{commands:?}");
        assert_eq!(commands[0].ports, [said(&dir)]);
        let stop = Request::CommandStop {
            thread,
            command: commands[0].id,
        };
        server.request(stop).await.unwrap();
    });
}

#[test]
fn a_server_reached_at_a_public_origin_says_no_port_of_this_computer_s_loopback() {
    let dir = folder("ports-public");
    block_on!(async {
        let mut setup = Setup::new(serves());
        setup.origin = PublicOrigin::parse("https://glave.example.com");
        let (server, _) = setup.build(&dir).await;
        let thread = start(&server, false).await;
        turn(&server, thread, "serve the page").await.unwrap();
        // It listens and is listed, as anywhere; a browser elsewhere could not reach the port.
        said(&dir);
        let commands = listed(&server, thread).await;
        assert_eq!(commands.len(), 1, "{commands:?}");
        assert_eq!(commands[0].ports, [] as [u16; 0]);
    });
}
