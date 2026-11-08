//! Where a thread's commands in the background listen, through the API: `command/list` gives
//! a command that serves on loopback its port, for the person on this computer, and none where a
//! browser of the caller's would be elsewhere, at a public origin; and a preview of it at its own
//! address only where glave's pages are opened at `localhost`, never at `127.0.0.1` itself.

use std::path::Path;

use glave_ports::host::PublicOrigin;
use glave_session::protocol::Request;
use glave_session::signin::{LocalToken, SignIn};
use serde_json::{Value, json};

use crate::support::issuer::fixed_clock;
use crate::support::web::{Served, Web, asked, serve_here, serve_with};
use crate::support::*;

/// A page and a server for it, on loopback at a port the system picks, as a dev server is,
/// which writes its port (`port`) once it listens.
const SERVES: &str = "printf 'hello' > index.html && exec python3 -c \"import http.server as h, os; \
     s = h.ThreadingHTTPServer(('127.0.0.1', 0), h.SimpleHTTPRequestHandler); \
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

/// What `command/list` answers over HTTP for a thread of a server in `dir` whose command
/// serves, the server served by `serve` with a local token's sign-in; and the port it serves on.
async fn listed_when_served<F>(dir: &Path, serve: impl FnOnce(Tested, SignIn) -> F) -> (Value, u16)
where
    F: Future<Output = Served>,
{
    let (server, _) = Setup::new(serves()).build(dir).await;
    let thread = start(&server, false).await;
    turn(&server, thread, "serve the page").await.unwrap();
    let token = LocalToken::new([3; 32]);
    let serving = serve(
        server,
        SignIn::local(token.clone(), Box::new(fixed_clock())),
    )
    .await;
    let mut here = Web::new(&serving, Some(token.reveal().to_owned()));
    let listed = asked(&mut here, &Request::CommandList { thread }).await;
    serving.close().await;
    (listed, said(dir))
}

#[test]
fn a_preview_is_the_server_s_own_address_where_glave_is_opened_at_localhost() {
    let dir = folder("previews-localhost");
    block_on!(async {
        // At both of this computer's loopback addresses, glave's pages are at `localhost`.
        let (listed, port) = listed_when_served(&dir, serve_here).await;
        assert_eq!(listed["previewed"], true, "{listed}");
        let command = &listed["commands"][0];
        assert_eq!(command["ports"], json!([port]));
        let url = format!("http://127.0.0.1:{port}/");
        assert_eq!(
            listed["previews"],
            json!([{"command": command["id"], "port": port, "url": url}])
        );
    });
}

#[test]
fn no_preview_where_glave_is_opened_at_127_0_0_1_itself() {
    let dir = folder("previews-address");
    block_on!(async {
        // A preview at 127.0.0.1 would be the same site as glave's own pages there.
        let serve = |server, signin| serve_with(server, signin, None);
        let (listed, port) = listed_when_served(&dir, serve).await;
        assert_eq!(listed["previewed"], false, "{listed}");
        assert_eq!(listed["previews"], json!([]));
        assert_eq!(listed["commands"][0]["ports"], json!([port]));
    });
}
