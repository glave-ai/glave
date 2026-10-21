//! Watching a thread (`thread/watch`) over every transport: in-process, a stream of lines as
//! stdio carries it, a Unix socket to a long-running server, and HTTP with server-sent events.
//! Rebuilding from `after` and then watching equals watching from the start; a watcher pushed
//! past its backlog still gets every event; one that leaves and comes back with `after` misses
//! nothing; and a watch ends when its thread is deleted, or when the client ends it.

use glave_event::{Body, Part, ThreadId, decode};
use glave_ports::Caller;
use glave_session::protocol::{Answer, Notification, Request, Update, code};
use glave_session::{Answering, TurnItem, Watch, Watched, rpc};
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader, Lines,
};
use tokio::net::UnixListener;
use tokio::task::JoinHandle;

use crate::support::issuer::{Issuer, Signer};
use crate::support::web::{Events, Served, Web, serve_http};
use crate::support::*;

/// The four transports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    InProcess,
    Stdio,
    Socket,
    Http,
}

const KINDS: [Kind; 4] = [Kind::InProcess, Kind::Stdio, Kind::Socket, Kind::Http];

/// A server reached over one transport, which makes watchers of it.
struct Over {
    kind: Kind,
    server: Tested,
    socket: Option<(std::path::PathBuf, JoinHandle<()>)>,
    http: Option<(Served, Issuer, String)>,
    /// Stdio connections, each the server's only client, kept until the test is done.
    stdio: Vec<JoinHandle<std::io::Result<()>>>,
}

impl Over {
    async fn new(kind: Kind, server: &Tested, name: &str) -> Self {
        let mut over = Self {
            kind,
            server: server.clone(),
            socket: None,
            http: None,
            stdio: Vec::new(),
        };
        match kind {
            Kind::Socket => {
                // Short, as socket paths must be.
                let path = std::path::PathBuf::from(format!(
                    "/tmp/glave-watch-{name}-{}.sock",
                    std::process::id()
                ));
                let _ = std::fs::remove_file(&path);
                let listener = UnixListener::bind(&path).unwrap();
                let serving = server.clone();
                let accepting = tokio::spawn(async move {
                    let mut connections = tokio::task::JoinSet::new();
                    while let Ok((stream, _)) = listener.accept().await {
                        let (read, write) = stream.into_split();
                        let server = serving.clone();
                        connections.spawn(rpc::serve_connection(
                            server,
                            BufReader::new(read),
                            write,
                        ));
                    }
                });
                over.socket = Some((path, accepting));
            }
            Kind::Http => {
                let issuer = Issuer::start().await;
                let signer = Signer::es256("key-1");
                issuer.publish(&signer);
                let http = serve_http(server.clone(), &issuer).await;
                let token = signer.sign(&issuer.claims("ada"));
                over.http = Some((http, issuer, token));
            }
            Kind::InProcess | Kind::Stdio => {}
        }
        over
    }

    /// A new connection to the server, watching nothing yet.
    async fn watcher(&mut self) -> Watcher {
        match self.kind {
            Kind::InProcess => Watcher::InProcess(self.server.clone(), None),
            Kind::Stdio => {
                let (client_out, server_in) = tokio::io::duplex(1 << 16);
                let (server_out, client_in) = tokio::io::duplex(1 << 16);
                let serving =
                    rpc::serve(self.server.clone(), BufReader::new(server_in), server_out);
                self.stdio.push(tokio::spawn(serving));
                Watcher::Lines(LinesWatcher::new(Box::new(client_out), Box::new(client_in)))
            }
            Kind::Socket => {
                let (path, _) = self.socket.as_ref().unwrap();
                let stream = tokio::net::UnixStream::connect(path).await.unwrap();
                let (read, write) = stream.into_split();
                Watcher::Lines(LinesWatcher::new(Box::new(write), Box::new(read)))
            }
            Kind::Http => {
                let (served, _, token) = self.http.as_ref().unwrap();
                Watcher::Http(Web::new(served, Some(token.clone())), None)
            }
        }
    }

    async fn close(self) {
        if let Some((path, accepting)) = self.socket {
            accepting.abort();
            let _ = std::fs::remove_file(path);
        }
        if let Some((served, _, _)) = self.http {
            served.close().await;
        }
        self.server.shutdown().await;
        for serving in self.stdio {
            serving.abort();
        }
    }
}

type Writer = Box<dyn AsyncWrite + Unpin + Send>;
type Reader = Box<dyn AsyncRead + Unpin + Send>;

/// JSON-RPC over one connection of lines.
struct LinesWatcher {
    to_server: Writer,
    from_server: Lines<BufReader<Reader>>,
    next_id: u64,
    /// The id of the `thread/watch` request whose notifications come.
    watching: Option<u64>,
}

impl LinesWatcher {
    fn new(to_server: Writer, from_server: Reader) -> Self {
        Self {
            to_server,
            from_server: BufReader::new(from_server).lines(),
            next_id: 0,
            watching: None,
        }
    }

    async fn send(&mut self, request: &Request) -> u64 {
        self.next_id += 1;
        let line = format!("{}\n", rpc::message(self.next_id, request));
        self.to_server.write_all(line.as_bytes()).await.unwrap();
        self.next_id
    }

    async fn receive(&mut self) -> Value {
        let line = self.from_server.next_line().await.unwrap().unwrap();
        serde_json::from_str(&line).unwrap()
    }
}

/// One client connection watching a thread.
enum Watcher {
    InProcess(Tested, Option<Watch>),
    Lines(LinesWatcher),
    Http(Web, Option<(Events, u64)>),
}

/// What a watch said: a notification, or its answer.
enum Said {
    Note(Notification),
    Answer(Value),
}

impl Watcher {
    /// Starts watching `thread` after `after`.
    async fn watch(&mut self, thread: ThreadId, after: Option<u64>) {
        let after = after.map(|after| glave_event::Seq::new(after).unwrap());
        let request = Request::ThreadWatch { thread, after };
        match self {
            Self::InProcess(server, watch) => {
                *watch = Some(
                    server
                        .watch(&Caller::default(), thread, after)
                        .await
                        .unwrap(),
                );
            }
            Self::Lines(lines) => lines.watching = Some(lines.send(&request).await),
            Self::Http(web, watch) => {
                let (message, id) = web.message(&request);
                let response = web.open(&message, Some("thread/watch")).await;
                assert_eq!(response.headers()["content-type"], "text/event-stream");
                *watch = Some((Events::new(response), id));
            }
        }
    }

    /// What the watch says next.
    async fn next(&mut self) -> Said {
        match self {
            Self::InProcess(_, watch) => match watch.as_mut().unwrap().next().await.unwrap() {
                Watched::Note(note) => Said::Note(note.as_ref().clone()),
                Watched::Ended(ended) => Said::Answer(json!({"result": {"ended": ended.unwrap()}})),
            },
            Self::Lines(lines) => {
                let message = lines.receive().await;
                if message.get("method").is_some() {
                    Said::Note(serde_json::from_value(message).unwrap())
                } else {
                    assert_eq!(Some(message["id"].as_u64().unwrap()), lines.watching);
                    Said::Answer(message)
                }
            }
            Self::Http(_, watch) => {
                let (events, id) = watch.as_mut().unwrap();
                let message = events.next().await.unwrap();
                if message.get("method").is_some() {
                    Said::Note(serde_json::from_value(message).unwrap())
                } else {
                    assert_eq!(message["id"], *id);
                    Said::Answer(message)
                }
            }
        }
    }

    /// Stops watching, as the client may: in-process by dropping the watch, over a connection
    /// of lines by `thread/unwatch`, which answers the watch, and over HTTP by closing the
    /// stream.
    async fn leave(&mut self, thread: ThreadId) {
        match self {
            Self::InProcess(_, watch) => *watch = None,
            Self::Lines(lines) => {
                let unwatch = lines.send(&Request::ThreadUnwatch { thread }).await;
                let (mut answered, mut done) = (false, false);
                while !(answered && done) {
                    let message = lines.receive().await;
                    if message.get("method").is_some() {
                        continue;
                    }
                    if Some(message["id"].as_u64().unwrap()) == lines.watching {
                        assert_eq!(message["result"], json!({"ended": "unwatched"}));
                        answered = true;
                    } else {
                        assert_eq!(message["id"], unwatch);
                        assert_eq!(message["result"], json!({}));
                        done = true;
                    }
                }
                lines.watching = None;
            }
            Self::Http(_, watch) => *watch = None,
        }
    }
}

/// Each event a watcher was sent, as `seq kind`, and every notification as one line.
#[derive(Default)]
struct Seen {
    events: Vec<String>,
    all: Vec<String>,
}

/// Reads `watcher` until it was sent `thread`'s event `last`.
async fn read_through(watcher: &mut Watcher, thread: ThreadId, last: u64, seen: &mut Seen) {
    read_until(watcher, thread, seen, |seq, _| seq == last).await;
}

/// Reads `watcher` until it was sent an event of `thread` that `wanted` takes: its `seq`.
async fn read_until(
    watcher: &mut Watcher,
    thread: ThreadId,
    seen: &mut Seen,
    wanted: impl Fn(u64, &str) -> bool,
) -> u64 {
    loop {
        let Said::Note(note) = watcher.next().await else {
            panic!("the watch ended before the event it waited for");
        };
        let Notification::Event { thread: of, event } = &note else {
            seen.all.push(format!("{note:?}"));
            continue;
        };
        if *of != thread {
            seen.all.push(format!("{note:?}"));
            continue;
        }
        let shown = format!("{} {}", event.seq.get(), event.kind.as_str());
        seen.events.push(shown.clone());
        seen.all.push(shown);
        if wanted(event.seq.get(), event.kind.as_str()) {
            return event.seq.get();
        }
    }
}

/// The thread's events, as `seq kind`.
async fn logged(server: &Tested, thread: ThreadId) -> Vec<String> {
    events(server, thread)
        .await
        .iter()
        .map(|event| format!("{} {}", event.seq.get(), event.kind.as_str()))
        .collect()
}

/// Runs a turn in-process, allowing what it asks.
async fn run(server: &Tested, thread: ThreadId, prompt: &str) {
    let mut stream = stream_turn(server, thread, prompt, None).await.unwrap();
    while let Some(item) = stream.next().await {
        match item {
            TurnItem::Note(note) => {
                if let Notification::Live {
                    update: Update::ApprovalPending { approval },
                    ..
                } = note.as_ref()
                {
                    let decide = Request::ApprovalDecide {
                        thread,
                        approval: approval.clone(),
                        decision: Answer::AllowOnce,
                    };
                    server.request(decide).await.unwrap();
                }
            }
            TurnItem::Missed(_) => {}
            TurnItem::Ended(ended) => {
                ended.unwrap();
                return;
            }
        }
    }
}

/// A server answering a reply, then a command it asks to run, then a reply.
async fn scripted(name: &str) -> (Tested, glave_ports::testing::TempDir) {
    let dir = folder(name);
    let answers = vec![
        answer(Some("Hello there."), None),
        answer(None, Some(("bash", json!({"command": "touch ran.txt"})))),
        answer(Some("Done."), None),
    ];
    (server(&dir, answers, Answering::Clients).await, dir)
}

fn last(log: &[String]) -> u64 {
    log.last()
        .unwrap()
        .split(' ')
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn rebuilding_from_after_then_watching_equals_watching_from_the_start() {
    block_on!(async {
        for kind in KINDS {
            let name = format!("rebuild-{kind:?}").to_lowercase();
            let (server, _dir) = scripted(&name).await;
            let mut over = Over::new(kind, &server, &name).await;
            let thread = start(&server, false).await;
            let mut first = over.watcher().await;
            first.watch(thread, None).await;
            run(&server, thread, "hi").await;
            let before = logged(&server, thread).await;
            let k = u64::try_from(before.len() / 2).unwrap();
            let mut second = over.watcher().await;
            second.watch(thread, Some(k)).await;
            run(&server, thread, "touch it").await;
            let log = logged(&server, thread).await;
            assert!(log.iter().any(|kind| kind.ends_with("approval.decided")));
            let (mut from_start, mut from_after) = (Seen::default(), Seen::default());
            read_through(&mut first, thread, last(&log), &mut from_start).await;
            read_through(&mut second, thread, last(&log), &mut from_after).await;
            assert_eq!(from_start.events, log, "{kind:?}");
            let rebuilt: Vec<String> = log[..usize::try_from(k).unwrap()]
                .iter()
                .chain(&from_after.events)
                .cloned()
                .collect();
            assert_eq!(rebuilt, log, "{kind:?}");
            // Both watched the second turn as it ran: the same events and live updates.
            let second_turn = |seen: &Seen| {
                let started = seen
                    .all
                    .iter()
                    .rposition(|shown| shown.ends_with("turn.started"))
                    .unwrap();
                seen.all[started..].to_vec()
            };
            assert_eq!(
                second_turn(&from_start),
                second_turn(&from_after),
                "{kind:?}"
            );
            over.close().await;
        }
    });
}

#[test]
fn a_watcher_pushed_past_its_backlog_still_gets_every_event() {
    block_on!(async {
        for kind in KINDS {
            let name = format!("behind-{kind:?}").to_lowercase();
            let dir = folder(&name);
            // Far more notifications than a watcher's backlog holds: one per part.
            let mut reply = answer(Some("start"), None);
            reply.parts = (0..3_000)
                .map(|n| Part::Text {
                    text: format!("part {n} "),
                })
                .collect();
            let server = server(&dir, vec![reply], Answering::Clients).await;
            let mut over = Over::new(kind, &server, &name).await;
            let thread = start(&server, false).await;
            let mut watcher = over.watcher().await;
            watcher.watch(thread, None).await;
            // The watcher reads nothing while the turn runs to its end.
            run(&server, thread, "go on").await;
            let log = logged(&server, thread).await;
            let mut seen = Seen::default();
            read_through(&mut watcher, thread, last(&log), &mut seen).await;
            assert_eq!(seen.events, log, "{kind:?}");
            over.close().await;
        }
    });
}

#[test]
fn a_watcher_that_comes_back_with_after_misses_nothing() {
    block_on!(async {
        for kind in KINDS {
            let name = format!("again-{kind:?}").to_lowercase();
            let dir = folder(&name);
            let gate = std::sync::Arc::new(tokio::sync::Notify::new());
            let mut setup = Setup::new(vec![answer(Some("Finished."), None)]);
            setup.gate = Some(std::sync::Arc::clone(&gate));
            let server = setup.build(&dir).await.0;
            let mut over = Over::new(kind, &server, &name).await;
            let thread = start(&server, false).await;
            let mut watcher = over.watcher().await;
            watcher.watch(thread, None).await;
            let mut stream = stream_turn(&server, thread, "go", None).await.unwrap();
            // The turn has started and taken the message; its answer waits at the gate.
            let mut seen = Seen::default();
            let had = read_until(&mut watcher, thread, &mut seen, |_, kind| {
                kind == "message.added"
            })
            .await;
            watcher.leave(thread).await;
            gate.notify_one();
            stream.finish().await.unwrap();
            let log = logged(&server, thread).await;
            // Back where it left off: the same connection over stdio, whose closing would stop
            // the server; a new one elsewhere.
            if kind != Kind::Stdio {
                watcher = over.watcher().await;
            }
            watcher.watch(thread, Some(had)).await;
            read_through(&mut watcher, thread, last(&log), &mut seen).await;
            assert_eq!(seen.events, log, "{kind:?}");
            over.close().await;
        }
    });
}

#[test]
fn deleting_the_thread_ends_its_watch() {
    block_on!(async {
        for kind in KINDS {
            let name = format!("deleted-{kind:?}").to_lowercase();
            let dir = folder(&name);
            let server = server(&dir, Vec::new(), Answering::Clients).await;
            let mut over = Over::new(kind, &server, &name).await;
            let thread = start(&server, false).await;
            let mut watcher = over.watcher().await;
            watcher.watch(thread, None).await;
            // Watching once its first event comes.
            let mut seen = Seen::default();
            read_through(&mut watcher, thread, 1, &mut seen).await;
            server
                .request(Request::ThreadDelete { thread })
                .await
                .unwrap();
            let ended = loop {
                match watcher.next().await {
                    Said::Note(Notification::Event { event, .. }) => {
                        let kind = decode(event).unwrap().body;
                        if let Body::ThreadDeleted(_) = kind {
                            let Said::Answer(ended) = watcher.next().await else {
                                panic!("the watch went on after its thread was deleted");
                            };
                            break ended;
                        }
                    }
                    Said::Note(_) => {}
                    Said::Answer(answer) => panic!("ended before the deletion: {answer}"),
                }
            };
            assert_eq!(ended["result"], json!({"ended": "deleted"}), "{kind:?}");
            // A deleted thread is not there to watch.
            let refused = server
                .watch(&Caller::default(), thread, None)
                .await
                .err()
                .unwrap();
            assert_eq!(refused.code, code::NO_THREAD);
            over.close().await;
        }
    });
}

#[test]
fn a_connection_of_lines_watches_each_thread_once_and_at_most_64() {
    block_on!(async {
        for kind in [Kind::Stdio, Kind::Socket] {
            let name = format!("most-{kind:?}").to_lowercase();
            let dir = folder(&name);
            let server = server(&dir, Vec::new(), Answering::Clients).await;
            let mut over = Over::new(kind, &server, &name).await;
            let Watcher::Lines(mut lines) = over.watcher().await else {
                unreachable!()
            };
            let mut threads = Vec::new();
            for _ in 0..65 {
                threads.push(start(&server, false).await);
            }
            for thread in &threads[..64] {
                lines
                    .send(&Request::ThreadWatch {
                        thread: *thread,
                        after: None,
                    })
                    .await;
            }
            // Each watch is held once it sends its thread's first event.
            let mut heard = std::collections::BTreeSet::new();
            while heard.len() < 64 {
                let message = lines.receive().await;
                if message["method"] == "thread/event" {
                    heard.insert(message["params"]["thread"].as_str().unwrap().to_owned());
                }
            }
            let refused = |message: &Value| message["error"]["code"].as_i64();
            let again = lines
                .send(&Request::ThreadWatch {
                    thread: threads[0],
                    after: None,
                })
                .await;
            let over_the_top = lines
                .send(&Request::ThreadWatch {
                    thread: threads[64],
                    after: None,
                })
                .await;
            let (mut twice, mut most) = (None, None);
            while twice.is_none() || most.is_none() {
                let message = lines.receive().await;
                if message["id"] == again {
                    twice = refused(&message);
                } else if message["id"] == over_the_top {
                    most = refused(&message);
                }
            }
            assert_eq!(twice, Some(code::INVALID_PARAMS), "{kind:?}");
            assert_eq!(most, Some(code::LIMIT), "{kind:?}");
            over.close().await;
        }
    });
}
