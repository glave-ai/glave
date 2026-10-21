//! Clients of a test server over each of the four transports (in-process, a stream of lines
//! as stdio carries it, a Unix socket to a long-running server, HTTP with server-sent events),
//! made to follow a thread: watching it, or a turn in it.

use glave_event::ThreadId;
use glave_ports::Caller;
use glave_session::protocol::{Answer, Notification, Request, Update};
use glave_session::{Answering, TurnItem, TurnStream, Watch, Watched, rpc};
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader, Lines,
};
use tokio::net::UnixListener;
use tokio::task::JoinHandle;

use super::issuer::{Issuer, Signer};
use super::requests::{events, stream_turn, turn_start};
use super::script::answer;
use super::setup::{Tested, folder, server};
use super::web::{Events, Served, Web, serve_http};

/// The four transports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    InProcess,
    Stdio,
    Socket,
    Http,
}

pub(crate) const KINDS: [Kind; 4] = [Kind::InProcess, Kind::Stdio, Kind::Socket, Kind::Http];

/// A server reached over one transport, which makes watchers of it.
pub(crate) struct Over {
    kind: Kind,
    server: Tested,
    socket: Option<(std::path::PathBuf, JoinHandle<()>)>,
    http: Option<(Served, Issuer, String)>,
    /// Stdio connections, each the server's only client, kept until the test is done.
    stdio: Vec<JoinHandle<std::io::Result<()>>>,
}

impl Over {
    pub(crate) async fn new(kind: Kind, server: &Tested, name: &str) -> Self {
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
    pub(crate) async fn watcher(&mut self) -> Watcher {
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

    /// A client of the server over HTTP, signed in.
    pub(crate) fn web(&self) -> Web {
        let (served, _, token) = self.http.as_ref().unwrap();
        Web::new(served, Some(token.clone()))
    }

    pub(crate) async fn close(self) {
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

pub(crate) type Writer = Box<dyn AsyncWrite + Unpin + Send>;
pub(crate) type Reader = Box<dyn AsyncRead + Unpin + Send>;

/// JSON-RPC over one connection of lines.
pub(crate) struct LinesWatcher {
    to_server: Writer,
    from_server: Lines<BufReader<Reader>>,
    next_id: u64,
    /// The id of the `thread/watch` request whose notifications come.
    pub(crate) watching: Option<u64>,
}

impl LinesWatcher {
    pub(crate) fn new(to_server: Writer, from_server: Reader) -> Self {
        Self {
            to_server,
            from_server: BufReader::new(from_server).lines(),
            next_id: 0,
            watching: None,
        }
    }

    pub(crate) async fn send(&mut self, request: &Request) -> u64 {
        self.next_id += 1;
        let line = format!("{}\n", rpc::message(self.next_id, request));
        self.to_server.write_all(line.as_bytes()).await.unwrap();
        self.next_id
    }

    pub(crate) async fn receive(&mut self) -> Value {
        let line = self.from_server.next_line().await.unwrap().unwrap();
        serde_json::from_str(&line).unwrap()
    }
}

/// One client connection following a thread: watching it, or a turn in it.
pub(crate) enum Watcher {
    InProcess(Tested, Option<InProcess>),
    Lines(LinesWatcher),
    Http(Web, Option<(Events, u64)>),
}

/// What a client in the server's process follows.
pub(crate) enum InProcess {
    Watch(Watch),
    Turn(TurnStream),
}

/// What a watch said: a notification, or its answer.
pub(crate) enum Said {
    Note(Notification),
    Answer(Value),
}

impl Watcher {
    /// Starts watching `thread` after `after`.
    pub(crate) async fn watch(&mut self, thread: ThreadId, after: Option<u64>) {
        let after = after.map(|after| glave_event::Seq::new(after).unwrap());
        let request = Request::ThreadWatch { thread, after };
        match self {
            Self::InProcess(server, watch) => {
                let caller = Caller::default();
                let watched = server.watch(&caller, thread, after).await.unwrap();
                *watch = Some(InProcess::Watch(watched));
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

    /// Starts a turn saying `prompt` in `thread` and follows it.
    pub(crate) async fn start_turn(&mut self, thread: ThreadId, prompt: &str) {
        let request = turn_start(thread, prompt, None);
        match self {
            Self::InProcess(server, following) => {
                let caller = Caller::default();
                let stream = server.start_streaming(&caller, request).await.unwrap();
                *following = Some(InProcess::Turn(stream));
            }
            Self::Lines(lines) => lines.watching = Some(lines.send(&request).await),
            Self::Http(web, following) => {
                let (message, id) = web.message(&request);
                let response = web.open(&message, Some("turn/start")).await;
                assert_eq!(response.headers()["content-type"], "text/event-stream");
                *following = Some((Events::new(response), id));
            }
        }
    }

    /// What the watch, or the turn, says next.
    pub(crate) async fn next(&mut self) -> Said {
        match self {
            Self::InProcess(_, following) => match following.as_mut().unwrap() {
                InProcess::Watch(watch) => match watch.next().await.unwrap() {
                    Watched::Note(note) => Said::Note(note.as_ref().clone()),
                    Watched::Ended(ended) => {
                        Said::Answer(json!({"result": {"ended": ended.unwrap()}}))
                    }
                },
                InProcess::Turn(turn) => match turn.next().await.unwrap() {
                    TurnItem::Note(note) => Said::Note(note.as_ref().clone()),
                    TurnItem::Missed(missed) => panic!("missed {missed} in the server's process"),
                    TurnItem::Ended(ended) => {
                        Said::Answer(json!({"result": {"turn": ended.unwrap()}}))
                    }
                },
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
    pub(crate) async fn leave(&mut self, thread: ThreadId) {
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
pub(crate) struct Followed {
    pub(crate) events: Vec<String>,
    pub(crate) all: Vec<String>,
}

/// Reads `watcher` until it was sent `thread`'s event `last`.
pub(crate) async fn read_through(
    watcher: &mut Watcher,
    thread: ThreadId,
    last: u64,
    seen: &mut Followed,
) {
    read_until(watcher, thread, seen, |seq, _| seq == last).await;
}

/// Reads `watcher` until it was sent an event of `thread` that `wanted` takes: its `seq`.
pub(crate) async fn read_until(
    watcher: &mut Watcher,
    thread: ThreadId,
    seen: &mut Followed,
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
pub(crate) async fn logged(server: &Tested, thread: ThreadId) -> Vec<String> {
    events(server, thread)
        .await
        .iter()
        .map(|event| format!("{} {}", event.seq.get(), event.kind.as_str()))
        .collect()
}

/// Runs a turn in-process, allowing what it asks.
pub(crate) async fn run(server: &Tested, thread: ThreadId, prompt: &str) {
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
pub(crate) async fn scripted(name: &str) -> (Tested, glave_ports::testing::TempDir) {
    let dir = folder(name);
    let answers = vec![
        answer(Some("Hello there."), None),
        answer(None, Some(("bash", json!({"command": "touch ran.txt"})))),
        answer(Some("Done."), None),
    ];
    (server(&dir, answers, Answering::Clients).await, dir)
}

pub(crate) fn last(log: &[String]) -> u64 {
    log.last()
        .unwrap()
        .split(' ')
        .next()
        .unwrap()
        .parse()
        .unwrap()
}
