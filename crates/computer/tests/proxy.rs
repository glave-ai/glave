//! The outbound proxy against a stand-in for the internet: names resolve to what each test
//! says, and the one public address a test serves at is a server on this computer, so nothing
//! leaves it. Every connection the proxy makes is recorded, to prove it connects only to an
//! address it checked and looks up only names it allowed.

use std::collections::BTreeMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use glave_computer::{Allowed, Held, Host, Internet, Limits, Mode, Network, Proxy, Rules};
use glave_ports::block_on;
use glave_ports::testing::any_secret;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::timeout;

/// A public address, served by the test's own server on this computer.
const PUBLIC: IpAddr = IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34));

/// The longest any step of a test waits.
const LIMIT: Duration = Duration::from_secs(10);

/// The internet as a test sees it: names and what they resolve to, and every lookup and
/// connection the proxy asked for.
#[derive(Clone)]
struct StandIn {
    names: BTreeMap<&'static str, Vec<IpAddr>>,
    looked_up: Arc<Mutex<Vec<String>>>,
    connected: Arc<Mutex<Vec<SocketAddr>>>,
}

impl StandIn {
    fn new() -> Self {
        let ip = |text: &str| text.parse::<IpAddr>().unwrap();
        let names = BTreeMap::from([
            ("allowed.example", vec![PUBLIC]),
            ("other.example", vec![PUBLIC]),
            ("metadata.example", vec![ip("169.254.169.254")]),
            ("mapped.example", vec![ip("::ffff:169.254.169.254")]),
            ("inside.example", vec![ip("10.0.0.5")]),
            ("here.example", vec![ip("127.0.0.1")]),
            ("both.example", vec![ip("169.254.169.254"), PUBLIC]),
        ]);
        Self {
            names,
            looked_up: Arc::default(),
            connected: Arc::default(),
        }
    }

    fn looked_up(&self) -> Vec<String> {
        self.looked_up.lock().unwrap().clone()
    }

    fn connected(&self) -> Vec<SocketAddr> {
        self.connected.lock().unwrap().clone()
    }
}

impl Network for StandIn {
    fn resolve(&self, name: &str) -> impl Future<Output = io::Result<Vec<IpAddr>>> + Send {
        self.looked_up.lock().unwrap().push(name.to_owned());
        std::future::ready(Ok(self.names.get(name).cloned().unwrap_or_default()))
    }

    async fn connect(&self, to: SocketAddr) -> io::Result<TcpStream> {
        self.connected.lock().unwrap().push(to);
        if to.ip() != PUBLIC {
            return Err(io::ErrorKind::ConnectionRefused.into());
        }
        TcpStream::connect((Ipv4Addr::LOCALHOST, to.port())).await
    }
}

/// A proxy and the server behind the public address, each a task the test owns.
struct Running {
    proxy: SocketAddr,
    server: u16,
    secret: String,
    _tasks: JoinSet<()>,
}

/// Limits for tests: every wait shorter than the test's own.
fn limits() -> Limits {
    Limits {
        hold: LIMIT,
        ..Limits::default()
    }
}

/// Starts a server at the public address that answers each connection's first bytes with what
/// it read, prefixed `got `, and a proxy over `network` of the rules `rules` makes of the
/// server's port.
async fn start<N: Network>(
    rules: impl FnOnce(u16) -> Rules,
    network: N,
    holds: Option<mpsc::Sender<Held>>,
    limits: Limits,
) -> Running {
    let mut tasks = JoinSet::new();
    let server = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = server.local_addr().unwrap().port();
    tasks.spawn(async move {
        loop {
            let (mut stream, _) = server.accept().await.unwrap();
            let mut read = vec![0_u8; 4096];
            let n = stream.read(&mut read).await.unwrap();
            let mut reply = b"got ".to_vec();
            reply.extend_from_slice(&read[..n]);
            stream.write_all(&reply).await.unwrap();
        }
    });
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let proxy = listener.local_addr().unwrap();
    let secret = any_secret();
    let serving = Proxy::new(rules(port), network, secret.as_str(), holds, limits);
    tasks.spawn(serving.serve(listener));
    Running {
        proxy,
        server: port,
        secret,
        _tasks: tasks,
    }
}

/// `request` with the proxy's credential, as a client given the proxy's URL sends it.
fn presenting(request: &str, secret: &str) -> String {
    let credential = BASE64.encode(format!("t:{secret}"));
    request.replacen(
        "\r\n",
        &format!("\r\nProxy-Authorization: Basic {credential}\r\n"),
        1,
    )
}

/// Sends `request` to the proxy with its credential, then `then` once the proxy has
/// answered, and returns all it sent back until it closed.
async fn ask(running: &Running, request: &str, then: &[u8]) -> String {
    exchange(running, &presenting(request, &running.secret), then).await
}

/// Sends `request` as it is, then `then` once the proxy has answered, and returns all it sent
/// back until it closed.
async fn exchange(running: &Running, request: &str, then: &[u8]) -> String {
    timeout(LIMIT, async {
        let mut stream = TcpStream::connect(running.proxy).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut answer = vec![0_u8; 4096];
        let n = stream.read(&mut answer).await.unwrap();
        let mut said = String::from_utf8_lossy(&answer[..n]).into_owned();
        if said.starts_with("HTTP/1.1 200 Connection established") && !then.is_empty() {
            stream.write_all(then).await.unwrap();
        }
        let mut rest = String::new();
        let _ = stream.read_to_string(&mut rest).await;
        said.push_str(&rest);
        said
    })
    .await
    .unwrap()
}

/// Sends `request`, which the proxy holds, answers it no, and checks it was refused.
async fn refused_by_the_person(running: &Running, request: &str, held: &mut mpsc::Receiver<Held>) {
    let asking = ask(running, request, b"");
    let answering = async {
        let asked = timeout(LIMIT, held.recv()).await.unwrap().unwrap();
        asked.answer(false);
    };
    let (said, ()) = tokio::join!(asking, answering);
    assert!(said.starts_with("HTTP/1.1 403 Forbidden\r\n"), "{said}");
}

fn connect(host: &str, port: u16) -> String {
    format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n\r\n")
}

fn guarded(patterns: &[&str]) -> Rules {
    Rules::new(
        Mode::Guarded,
        patterns.iter().map(|p| Allowed::parse(p).unwrap()),
    )
}

fn open(_: u16) -> Rules {
    Rules::new(Mode::Open, None)
}

#[test]
fn an_allowed_host_is_tunnelled_to_the_address_the_proxy_checked() {
    block_on!(async {
        let net = StandIn::new();
        let running = start(
            |port| guarded(&[&format!("*.example:{port}")]),
            net.clone(),
            None,
            limits(),
        )
        .await;
        let said = ask(
            &running,
            &connect("allowed.example", running.server),
            b"hello",
        )
        .await;
        assert_eq!(said, "HTTP/1.1 200 Connection established\r\n\r\ngot hello");
        assert_eq!(net.connected(), [SocketAddr::new(PUBLIC, running.server)]);
    });
}

#[test]
fn plain_http_goes_to_the_host_decided_on_in_origin_form() {
    block_on!(async {
        let net = StandIn::new();
        let running = start(open, net.clone(), None, limits()).await;
        let port = running.server;
        let request = format!(
            "GET http://allowed.example:{port}/path HTTP/1.1\r\nHost: evil.example\r\n\r\n"
        );
        let said = ask(&running, &request, b"").await;
        assert_eq!(
            said,
            format!(
                "got GET /path HTTP/1.1\r\nHost: allowed.example:{port}\r\n\
                 Connection: close\r\n\r\n"
            )
        );
        assert_eq!(net.looked_up(), ["allowed.example"]);
    });
}

#[test]
fn a_host_not_allowed_is_refused_and_never_looked_up() {
    block_on!(async {
        let net = StandIn::new();
        let running = start(
            |_| guarded(&["allowed.example"]),
            net.clone(),
            None,
            limits(),
        )
        .await;
        let said = ask(&running, &connect("other.example", 443), b"").await;
        assert!(said.starts_with("HTTP/1.1 403 Forbidden\r\n"), "{said}");
        assert!(
            said.ends_with("no one allowed it; ask the person to allow it."),
            "{said}"
        );
        let off = start(
            |_| Rules::new(Mode::Offline, None),
            net.clone(),
            None,
            limits(),
        )
        .await;
        let said = ask(&off, &connect("allowed.example", 443), b"").await;
        assert!(
            said.ends_with("The network is off for commands here."),
            "{said}"
        );
        assert_eq!(net.looked_up(), [] as [String; 0], "no name leaves by DNS");
        assert_eq!(net.connected(), [] as [SocketAddr; 0]);
    });
}

#[test]
fn private_networks_and_cloud_metadata_are_withheld_whatever_name_leads_there() {
    block_on!(async {
        let net = StandIn::new();
        let running = start(open, net.clone(), None, limits()).await;
        for host in [
            "metadata.example",
            "mapped.example",
            "inside.example",
            "here.example",
            "169.254.169.254",
            "[fd00:ec2::254]",
            "[::ffff:a9fe:a9fe]",
            "10.0.0.5",
        ] {
            let said = ask(&running, &connect(host, 80), b"").await;
            assert!(
                said.starts_with("HTTP/1.1 403 Forbidden\r\n"),
                "{host}: {said}"
            );
            assert!(
                said.contains("withheld from commands on purpose"),
                "{host}: {said}"
            );
        }
        let said = ask(
            &running,
            "GET http://metadata.example/latest HTTP/1.1\r\n\r\n",
            b"",
        )
        .await;
        assert!(said.starts_with("HTTP/1.1 403 Forbidden\r\n"), "{said}");
        assert!(
            net.connected().is_empty(),
            "nothing withheld was connected to"
        );
        // A name that leads to both reaches only the public address.
        let said = ask(&running, &connect("both.example", running.server), b"hi").await;
        assert!(said.ends_with("got hi"), "{said}");
        assert_eq!(net.connected(), [SocketAddr::new(PUBLIC, running.server)]);
    });
}

#[test]
fn this_computers_own_names_are_withheld_through_the_internet_too() {
    block_on!(async {
        let running = start(open, Internet, None, limits()).await;
        let said = ask(&running, &connect("localhost", running.server), b"").await;
        assert!(said.starts_with("HTTP/1.1 403 Forbidden\r\n"), "{said}");
    });
}

#[test]
fn a_held_host_waits_for_the_persons_answer() {
    block_on!(async {
        let net = StandIn::new();
        let (holds, mut held) = mpsc::channel::<Held>(4);
        let running = start(|_| guarded(&[]), net.clone(), Some(holds), limits()).await;
        let request = connect("allowed.example", running.server);
        let asking = ask(&running, &request, b"yes");
        let answering = async {
            let asked = timeout(LIMIT, held.recv()).await.unwrap().unwrap();
            assert_eq!(asked.host(), &Host::parse("allowed.example").unwrap());
            assert!(
                net.looked_up().is_empty(),
                "nothing is looked up while held"
            );
            asked.answer(true);
        };
        let (said, ()) = tokio::join!(asking, answering);
        assert!(said.ends_with("got yes"), "{said}");
        refused_by_the_person(&running, &request, &mut held).await;
        let asking = ask(&running, &request, b"");
        let unanswered = async { drop(timeout(LIMIT, held.recv()).await.unwrap()) };
        let (said, ()) = tokio::join!(asking, unanswered);
        assert!(said.starts_with("HTTP/1.1 403 Forbidden\r\n"), "{said}");
        assert_eq!(
            net.looked_up(),
            ["allowed.example"],
            "only the yes was looked up"
        );
    });
}

#[test]
fn what_is_not_a_proxy_request_is_answered_so() {
    block_on!(async {
        let running = start(open, StandIn::new(), None, limits()).await;
        let said = ask(
            &running,
            "GET https://allowed.example/ HTTP/1.1\r\n\r\n",
            b"",
        )
        .await;
        assert!(said.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{said}");
        // Sent whole, so the proxy has read all of it when it answers.
        let huge = format!("GET / HTTP/1.1\r\nX: {}", "a".repeat(17_000));
        let said = ask(&running, &huge, b"").await;
        assert!(said.ends_with("The request's head is too large."), "{said}");
    });
}

#[test]
fn only_a_client_with_the_proxys_credential_is_served() {
    block_on!(async {
        let net = StandIn::new();
        let running = start(open, net.clone(), None, limits()).await;
        let request = connect("allowed.example", running.server);
        for said in [
            exchange(&running, &request, b"").await,
            exchange(&running, &presenting(&request, &any_secret()), b"").await,
        ] {
            assert!(
                said.starts_with("HTTP/1.1 407 Proxy Authentication Required\r\n"),
                "{said}"
            );
            assert!(said.contains("\r\nProxy-Authenticate: Basic"), "{said}");
        }
        assert_eq!(net.looked_up(), [] as [String; 0]);
    });
}

#[test]
fn a_guarded_host_goes_only_at_the_webs_ports_or_the_one_allowed() {
    block_on!(async {
        let net = StandIn::new();
        let running = start(
            |_| guarded(&["allowed.example"]),
            net.clone(),
            None,
            limits(),
        )
        .await;
        // The server is at neither 80 nor 443, and the entry names no port.
        let said = ask(&running, &connect("allowed.example", running.server), b"").await;
        assert!(said.starts_with("HTTP/1.1 403 Forbidden\r\n"), "{said}");
        assert_eq!(net.looked_up(), [] as [String; 0]);
    });
}

#[test]
fn held_connections_have_places_of_their_own_and_end_when_their_command_goes() {
    block_on!(async {
        let net = StandIn::new();
        let (holds, mut held) = mpsc::channel::<Held>(4);
        let tight = Limits {
            connections: 1,
            holds: 1,
            ..limits()
        };
        let allowed = |port| guarded(&[&format!("allowed.example:{port}")]);
        let running = start(allowed, net.clone(), Some(holds), tight).await;
        let mut waiting = TcpStream::connect(running.proxy).await.unwrap();
        let other = presenting(&connect("other.example", 443), &running.secret);
        waiting.write_all(other.as_bytes()).await.unwrap();
        let mut asked = timeout(LIMIT, held.recv()).await.unwrap().unwrap();
        assert_eq!(asked.port(), 443);
        let said = ask(&running, &connect("other.example", 443), b"").await;
        assert!(
            said.starts_with("HTTP/1.1 503 Service Unavailable\r\n"),
            "{said}"
        );
        let said = ask(&running, &connect("allowed.example", running.server), b"in").await;
        assert!(
            said.ends_with("got in"),
            "a held connection crowds out no other: {said}"
        );
        drop(waiting);
        timeout(LIMIT, asked.withdrawn()).await.unwrap();
        // Its place among the holds is free again.
        let again = connect("other.example", 443);
        refused_by_the_person(&running, &again, &mut held).await;
    });
}
