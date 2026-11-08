//! The previews of what a thread's commands in the background serve, through the API: a
//! command that listens is listed with a preview at an address of its own on this computer,
//! which serves its page; the preview closes when the command is stopped, ends by itself, or
//! the thread is archived; and none is shown where a browser of the caller's would not reach
//! it, at a public origin.

use std::path::Path;
use std::time::{Duration, Instant};

use glave_event::{Body, ThreadId, decode};
use glave_ports::Caller;
use glave_ports::host::PublicOrigin;
use glave_session::Watched;
use glave_session::protocol::{Notification, Preview, Reply, Request};
use serde_json::json;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;

use crate::support::*;

/// A page and a server for it, on loopback at a port the system picks, as a dev server is,
/// which says its own number (`server.pid`) and, once it listens, its port (`port`).
const SERVES: &str = "printf 'hello from the preview' > index.html && echo $$ > server.pid && \
     exec python3 -c \"import http.server as h, os; \
     s = h.ThreadingHTTPServer(('127.0.0.1', 0), h.SimpleHTTPRequestHandler); \
     open('port.next', 'w').write(str(s.server_address[1])); os.rename('port.next', 'port'); \
     s.serve_forever()\"";

/// How long the server may take to listen, and a step over loopback to answer.
const SOON: Duration = Duration::from_secs(20);

fn serves() -> Vec<glave_ports::Completion> {
    vec![
        answer(
            None,
            Some(("bash", json!({"command": SERVES, "background": true}))),
        ),
        answer(Some("It is serving."), None),
    ]
}

/// What `command/list` answers for `thread`.
async fn listed<L: glave_ports::EventLog + 'static>(
    server: &Tested<L>,
    thread: ThreadId,
) -> (Vec<glave_harness::RunningCommand>, Vec<Preview>) {
    match server
        .request(Request::CommandList { thread })
        .await
        .unwrap()
    {
        Reply::Commands { commands, previews } => (commands, previews),
        other => panic!("commands, not {other:?}"),
    }
}

/// The port the server in `dir` listens on, once it does.
async fn listening(dir: &Path) -> u16 {
    let began = Instant::now();
    loop {
        if let Ok(port) = std::fs::read_to_string(dir.join("port")) {
            return port.parse().unwrap();
        }
        assert!(began.elapsed() < SOON, "the server did not listen soon");
        tokio::task::yield_now().await;
    }
}

/// The preview of `thread`'s one command, whose server in `dir` listens.
async fn previewed<L: glave_ports::EventLog + 'static>(
    server: &Tested<L>,
    thread: ThreadId,
    dir: &Path,
) -> Preview {
    let port = listening(dir).await;
    let (commands, previews) = listed(server, thread).await;
    assert_eq!(commands.len(), 1, "{commands:?}");
    let [preview] = previews.as_slice() else {
        panic!("one preview of the server listening on {port}, not {previews:?}");
    };
    assert_eq!((preview.command, preview.port), (commands[0].id, port));
    preview.clone()
}

/// The port of a preview's own address.
fn port_of(preview: &Preview) -> u16 {
    let at = preview.url.strip_prefix("http://127.0.0.1:").unwrap();
    at.strip_suffix('/').unwrap().parse().unwrap()
}

/// What a browser asking for `/` at `port` gets.
async fn page(port: u16) -> std::io::Result<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await?;
    let asked = format!("GET / HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n");
    stream.write_all(asked.as_bytes()).await?;
    let mut answered = String::new();
    tokio::time::timeout(SOON, stream.read_to_string(&mut answered))
        .await
        .map_err(|_| std::io::ErrorKind::TimedOut)??;
    Ok(answered)
}

#[test]
fn a_command_that_listens_has_a_preview_of_its_own_until_it_is_stopped() {
    let dir = folder("previews-stopped");
    block_on!(async {
        let (server, _) = Setup::new(serves()).build(&dir).await;
        let thread = start(&server, false).await;
        turn(&server, thread, "serve the page").await.unwrap();
        let preview = previewed(&server, thread, &dir).await;
        let at = port_of(&preview);
        assert_ne!(at, preview.port, "an address of its own, never the command's");
        let answered = page(at).await.unwrap();
        assert!(answered.starts_with("HTTP/1.0 200"), "{answered}");
        assert!(answered.ends_with("hello from the preview"), "{answered}");
        assert_eq!(
            previewed(&server, thread, &dir).await,
            preview,
            "the same preview each time it is listed"
        );

        let stop = Request::CommandStop {
            thread,
            command: preview.command,
        };
        server.request(stop).await.unwrap();
        assert!(page(at).await.is_err(), "a stopped command's preview is closed");
        assert_eq!(listed(&server, thread).await, (Vec::new(), Vec::new()));
    });
}

#[test]
fn a_preview_closes_when_its_command_ends_by_itself() {
    let dir = folder("previews-ended");
    block_on!(async {
        let kill = "kill \"$(cat server.pid)\"";
        let mut answers = serves();
        answers.push(answer(None, Some(("bash", json!({"command": kill})))));
        answers.push(answer(Some("Stopped it."), None));
        let (server, _) = Setup::new(answers).build(&dir).await;
        let thread = start(&server, false).await;
        turn(&server, thread, "serve the page").await.unwrap();
        let at = port_of(&previewed(&server, thread, &dir).await);
        assert!(page(at).await.is_ok());

        let mut watching = server
            .watch(&Caller::default(), thread, None)
            .await
            .unwrap();
        turn(&server, thread, "now stop it").await.unwrap();
        tokio::time::timeout(SOON, async {
            loop {
                let Some(Watched::Note(note)) = watching.next().await else {
                    panic!("the watch ended before the command did")
                };
                if let Notification::Event { event, .. } = note.as_ref()
                    && let Body::CommandEnded(_) = decode(event.clone()).unwrap().body
                {
                    break;
                }
            }
        })
        .await
        .expect("the killed server is recorded as ended within seconds");
        assert!(page(at).await.is_err(), "its preview closed as it ended");
    });
}

#[test]
fn archiving_a_thread_closes_its_previews() {
    let dir = folder("previews-archived");
    block_on!(async {
        let (server, _) = Setup::new(serves()).build(&dir).await;
        let thread = start(&server, false).await;
        turn(&server, thread, "serve the page").await.unwrap();
        let at = port_of(&previewed(&server, thread, &dir).await);
        server
            .request(Request::ThreadArchive { thread })
            .await
            .unwrap();
        assert!(page(at).await.is_err());
    });
}

#[test]
fn a_server_reached_at_a_public_origin_shows_no_preview_of_this_computer_s_loopback() {
    let dir = folder("previews-public");
    block_on!(async {
        let mut setup = Setup::new(serves());
        setup.origin = PublicOrigin::parse("https://glave.example.com");
        let (server, _) = setup.build(&dir).await;
        let thread = start(&server, false).await;
        turn(&server, thread, "serve the page").await.unwrap();
        // It listens and is listed, as anywhere; a browser elsewhere could not open its preview.
        listening(&dir).await;
        let (commands, previews) = listed(&server, thread).await;
        assert_eq!(commands.len(), 1);
        assert_eq!(previews, []);
    });
}
