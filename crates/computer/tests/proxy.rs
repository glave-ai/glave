//! The outbound proxy against a stand-in for the internet: names resolve to what each test
//! says, and the one public address a test serves at is a server on this computer, so nothing
//! leaves it. Every connection the proxy makes is recorded, to prove it connects only to an
//! address it checked and looks up only names it allowed.

use std::collections::BTreeMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use glave_computer::{Allowed, Held, Host, Internet, Limits, Mode, Network, Proxy, Rules};
use glave_ports::block_on;
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
    _tasks: JoinSet<()>,
}

/// Starts a proxy of `rules` over `network`, and a server at the public address that answers
/// each connection's first bytes with what it read, prefixed `got `.
async fn start<N: Network>(rules: Rules, network: N, holds: Option<mpsc::Sender<Held>>) -> Running {
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
    let limits = Limits {
        hold: LIMIT,
        ..Limits::default()
    };
    tasks.spawn(Proxy::new(rules, network, holds, limits).serve(listener));
    Running {
        proxy,
        server: port,
        _tasks: tasks,
    }
}

/// Sends `request` to the proxy, then `then` once the proxy has answered, and returns all it
/// sent back until it closed.
async fn ask(running: &Running, request: &str, then: &[u8]) -> String {
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

fn connect(host: &str, port: u16) -> String {
    format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n\r\n")
}

fn limited(patterns: &[&str]) -> Rules {
    Rules::new(
        Mode::Limited,
        patterns.iter().map(|p| Allowed::parse(p).unwrap()),
    )
}

#[test]
fn an_allowed_host_is_tunnelled_to_the_address_the_proxy_checked() {
    block_on!(async {
        let net = StandIn::new();
        let running = start(limited(&["*.example"]), net.clone(), None).await;
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
        let running = start(Rules::new(Mode::Full, None), net.clone(), None).await;
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
        let running = start(limited(&["allowed.example"]), net.clone(), None).await;
        let said = ask(&running, &connect("other.example", 443), b"").await;
        assert!(said.starts_with("HTTP/1.1 403 Forbidden\r\n"), "{said}");
        assert!(
            said.ends_with("no one allowed it; ask the person to allow it."),
            "{said}"
        );
        let off = start(Rules::new(Mode::None, None), net.clone(), None).await;
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
        let running = start(Rules::new(Mode::Full, None), net.clone(), None).await;
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
        let running = start(Rules::new(Mode::Full, None), Internet, None).await;
        let said = ask(&running, &connect("localhost", running.server), b"").await;
        assert!(said.starts_with("HTTP/1.1 403 Forbidden\r\n"), "{said}");
    });
}

#[test]
fn a_held_host_waits_for_the_persons_answer() {
    block_on!(async {
        let net = StandIn::new();
        let (holds, mut held) = mpsc::channel::<Held>(4);
        let running = start(limited(&[]), net.clone(), Some(holds)).await;
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
        let asking = ask(&running, &request, b"");
        let answering = async {
            timeout(LIMIT, held.recv())
                .await
                .unwrap()
                .unwrap()
                .answer(false);
        };
        let (said, ()) = tokio::join!(asking, answering);
        assert!(said.starts_with("HTTP/1.1 403 Forbidden\r\n"), "{said}");
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
        let running = start(Rules::new(Mode::Full, None), StandIn::new(), None).await;
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
