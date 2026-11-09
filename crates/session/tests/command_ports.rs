//! Where a thread's commands in the background listen, through the API: `command/list` gives
//! a command that serves on loopback its ports, with a sentence, for the person on this computer,
//! and none where a browser of the caller's would be elsewhere, at a public origin; and a preview
//! of each at the server's own address only where glave's pages are opened at `localhost`, never
//! at `127.0.0.1` itself: at `127.0.0.1` for a server on every address of both families, as
//! Node's and Python's listen by default.

use std::path::Path;

use glave_ports::host::PublicOrigin;
use glave_session::protocol::Request;
use glave_session::signin::{LocalToken, SignIn};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;

use crate::support::issuer::fixed_clock;
use crate::support::web::{Served, Web, asked, serve_here, serve_with};
use crate::support::*;

/// A page's server, as a dev server is: at the address its argument names, `::` meaning every
/// address of both families, at a port the system picks, which it writes (`port`) once it
/// listens. It binds as a `TCPServer`, since `HTTPServer` asks DNS for its host's name as it
/// binds, which outlasted the wait on macOS runners.
const SERVER: &str = "\
import http.server as h, os, socket, socketserver, sys
both = sys.argv[1] == '::'
class Server(h.ThreadingHTTPServer):
    address_family = socket.AF_INET6 if both else socket.AF_INET
    def server_bind(self):
        if both:
            self.socket.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 0)
        socketserver.TCPServer.server_bind(self)
served = Server((sys.argv[1], 0), h.SimpleHTTPRequestHandler)
open('port.next', 'w').write(str(served.server_address[1]))
os.rename('port.next', 'port')
served.serve_forever()
";

/// Waits, within its own time limit, until the server listens.
const LISTENS: &str = "while [ ! -s port ]; do sleep 0.05; done";

/// The model starts the server in `dir` at `address`, then waits for it to listen, so the turn
/// ends once it does.
fn serves(dir: &Path, address: &str) -> Vec<glave_ports::Completion> {
    std::fs::write(dir.join("serve.py"), SERVER).unwrap();
    let serves = format!("printf 'hello' > index.html && exec python3 serve.py {address}");
    vec![
        answer(
            None,
            Some(("bash", json!({"command": serves, "background": true}))),
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
        let (server, _) = Setup::new(serves(&dir, "127.0.0.1")).build(&dir).await;
        let thread = start(&server, false).await;
        turn(&server, thread, "serve the page").await.unwrap();
        let commands = listed(&server, thread).await;
        assert_eq!(commands.len(), 1, "{commands:?}");
        let port = said(&dir);
        let ports: Vec<u16> = commands[0].ports.iter().map(|each| each.port).collect();
        assert_eq!(ports, [port]);
        assert_eq!(
            commands[0].listens.as_deref(),
            Some(format!("Listening on {port}.").as_str())
        );
        // Not reached over HTTP here, so nothing is previewed, nor said of a frame.
        assert_eq!(commands[0].ports[0].url, None);
        assert_eq!(commands[0].ports[0].unframed, None);
    });
}

#[test]
fn a_server_reached_at_a_public_origin_says_no_port_of_this_computer_s_loopback() {
    let dir = folder("ports-public");
    block_on!(async {
        let mut setup = Setup::new(serves(&dir, "127.0.0.1"));
        setup.origin = PublicOrigin::parse("https://glave.example.com");
        let (server, _) = setup.build(&dir).await;
        let thread = start(&server, false).await;
        turn(&server, thread, "serve the page").await.unwrap();
        // It listens and is listed, as anywhere; a browser elsewhere could not reach the port.
        said(&dir);
        let commands = listed(&server, thread).await;
        assert_eq!(commands.len(), 1, "{commands:?}");
        assert_eq!(commands[0].ports, []);
        assert_eq!(commands[0].listens, None);
    });
}

/// What `command/list` answers over HTTP for a thread of a server in `dir` whose command
/// serves at `address`, the server served by `serve` with a local token's sign-in; the
/// port it serves on; and the serving, which the test closes, which stops the command too.
async fn listed_when_served<F>(
    dir: &Path,
    address: &str,
    serve: impl FnOnce(Tested, SignIn) -> F,
) -> (Value, u16, Served)
where
    F: Future<Output = Served>,
{
    let (server, _) = Setup::new(serves(dir, address)).build(dir).await;
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
    (listed, said(dir), serving)
}

/// What a browser asking for `/` at `url` gets.
async fn page(url: &str) -> String {
    let at = url.strip_prefix("http://").unwrap().trim_end_matches('/');
    let mut stream = TcpStream::connect(at).await.unwrap();
    let asked = format!("GET / HTTP/1.0\r\nHost: {at}\r\n\r\n");
    stream.write_all(asked.as_bytes()).await.unwrap();
    let mut answered = String::new();
    stream.read_to_string(&mut answered).await.unwrap();
    answered
}

#[test]
fn a_preview_is_the_server_s_own_address_where_glave_is_opened_at_localhost() {
    let dir = folder("previews-localhost");
    block_on!(async {
        // At both of this computer's loopback addresses, glave's pages are at `localhost`.
        let (listed, port, serving) = listed_when_served(&dir, "127.0.0.1", serve_here).await;
        serving.close().await;
        assert_eq!(listed["previewed"], true, "{listed}");
        let url = format!("http://127.0.0.1:{port}/");
        assert_eq!(
            listed["commands"][0]["ports"],
            json!([{"port": port, "url": url, "framed": true}])
        );
        assert_eq!(
            listed["commands"][0]["listens"],
            format!("Listening on {port}.")
        );
    });
}

#[test]
fn a_server_on_every_address_of_both_families_is_previewed_and_framed_at_127_0_0_1() {
    let dir = folder("previews-both");
    block_on!(async {
        let (listed, port, serving) = listed_when_served(&dir, "::", serve_here).await;
        let url = format!("http://127.0.0.1:{port}/");
        assert_eq!(
            listed["commands"][0]["ports"],
            json!([{"port": port, "url": url, "framed": true}]),
            "{listed}"
        );
        let answered = page(&url).await;
        assert!(answered.starts_with("HTTP/1.0 200"), "{answered}");
        assert!(answered.ends_with("hello"), "{answered}");
        serving.close().await;
    });
}

#[test]
fn no_preview_where_glave_is_opened_at_127_0_0_1_itself() {
    let dir = folder("previews-address");
    block_on!(async {
        // A preview at 127.0.0.1 would be the same site as glave's own pages there.
        let serve = |server, signin| serve_with(server, signin, None);
        let (listed, port, serving) = listed_when_served(&dir, "127.0.0.1", serve).await;
        serving.close().await;
        assert_eq!(listed["previewed"], false, "{listed}");
        assert_eq!(
            listed["commands"][0]["ports"],
            json!([{"port": port, "framed": false}])
        );
    });
}
