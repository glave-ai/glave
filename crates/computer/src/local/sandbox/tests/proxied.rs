//! Commands kept to the outbound proxy, in the real sandbox: on Linux in their thread's network
//! namespace, whose one way out is glave's forwarder to the proxy, on macOS reaching only this
//! computer's loopback, where the proxy listens. The proxy reaches a stand-in for the internet
//! (names resolve to what these tests say, and the one public address is a server on this
//! computer), so nothing leaves it. On Linux they need the built glave as the forwarder
//! (`GLAVE_TEST_SANDBOX_HELPER`), and must find it where `GLAVE_REQUIRE_SANDBOX` is set.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::time::Duration;

use glave_ports::block_on;
use glave_ports::computer::{
    Command, ComputerError, Computers, Ending, Environment, NetworkMode, Sandbox, SandboxLevel,
    SandboxNetwork, Sources, WorkspaceId,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use super::{Collect, enforced, scratch};
use crate::local::sandbox::Outbound;
use crate::proxy::{Network, Rules};
use crate::{CancelSwitch, LocalComputers};

/// A public address, served by the test's own server on this computer.
const PUBLIC: IpAddr = IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34));

/// What the test's server answers every request with.
const HELLO: &str = "hello from the stand-in host";

/// The internet as these tests see it: a few names, and the one public address, which is the
/// test's server on this computer's loopback.
#[derive(Clone)]
struct StandIn {
    names: BTreeMap<&'static str, IpAddr>,
}

impl StandIn {
    fn new() -> Self {
        let names = BTreeMap::from([
            ("allowed.example", PUBLIC),
            (
                "metadata.example",
                IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)),
            ),
            ("inside.example", IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5))),
        ]);
        Self { names }
    }
}

impl Network for StandIn {
    fn resolve(&self, name: &str) -> impl Future<Output = io::Result<Vec<IpAddr>>> + Send {
        std::future::ready(Ok(self.names.get(name).copied().into_iter().collect()))
    }

    async fn connect(&self, to: SocketAddr) -> io::Result<TcpStream> {
        if to.ip() != PUBLIC {
            return Err(io::ErrorKind::ConnectionRefused.into());
        }
        TcpStream::connect((Ipv4Addr::LOCALHOST, to.port())).await
    }
}

/// What these tests run with where they run: on Linux, the built glave, which forwards in each
/// command's namespace; on macOS, nothing.
struct Kept {
    helper: Option<PathBuf>,
}

/// Where these tests run: where the sandbox is, and on Linux, with the built glave.
fn kept() -> Option<Kept> {
    if !enforced() {
        return None;
    }
    if cfg!(target_os = "macos") {
        return Some(Kept { helper: None });
    }
    let helper = std::env::var_os("GLAVE_TEST_SANDBOX_HELPER").map(PathBuf::from);
    assert!(
        helper.is_some() || std::env::var_os("GLAVE_REQUIRE_SANDBOX").is_none(),
        "GLAVE_REQUIRE_SANDBOX is set but GLAVE_TEST_SANDBOX_HELPER, the built glave, is not"
    );
    helper.map(|helper| Kept {
        helper: Some(helper),
    })
}

/// This computer's commands, reaching the stand-in internet through their proxies, with `dir`
/// as the home folder, forwarding through `helper` on Linux.
fn computers(dir: &Path, helper: Option<&Path>) -> LocalComputers {
    let environment: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(name, _)| name == "PATH")
        .chain([("HOME".into(), dir.as_os_str().to_owned())])
        .collect();
    let computers = LocalComputers::new(dir.join("copies"), environment).with_outbound(
        Outbound::over(Rules::new(NetworkMode::Open, []), StandIn::new()),
    );
    match helper {
        Some(helper) => computers.with_helper(helper),
        None => computers,
    }
}

/// A thread of its own: a workspace on `dir/app`, so a sandbox and a proxy of its own.
async fn thread(computers: &LocalComputers, dir: &Path) -> Result<WorkspaceId, ComputerError> {
    let computer = computers.create(&Environment::host()).await?;
    let sources = Sources::Folder {
        path: dir.join("app").to_string_lossy().into_owned(),
    };
    Ok(computers.open(computer.id, &sources).await?.id)
}

/// `script` in the `project` sandbox with `network`, with `dir` as the home folder.
fn sandboxed(dir: &Path, network: SandboxNetwork, script: &str) -> Command {
    let mut command = Command::new(script);
    command.sandbox = Some(Sandbox {
        level: SandboxLevel::Project,
        network,
        home: Some(dir.to_owned()),
        glave_home: Some(dir.join(".glave")),
    });
    command.timeout_secs = Some(60);
    command
}

/// Runs `command` in `workspace` to its end, and says what it printed.
async fn said(
    computers: &LocalComputers,
    workspace: WorkspaceId,
    command: &Command,
) -> Result<String, ComputerError> {
    let output = Collect::default();
    let finished = computers
        .run(workspace, command, &output, &CancelSwitch::default())
        .await?;
    let printed = output.0.lock().unwrap().clone();
    assert!(
        !matches!(finished.ending, Ending::TimedOut { .. }),
        "{}: {printed}",
        command.script
    );
    Ok(printed)
}

/// Runs each of `scripts` in `dir/app`, in the `project` sandbox with `network`, kept to the
/// proxy or not, while an HTTP server answers [`HELLO`] on loopback at the port each script's
/// `$SERVER` names. Each script runs in the thread its number names. Returns what each printed.
fn run(
    dir: &Path,
    helper: Option<&Path>,
    network: SandboxNetwork,
    scripts: &[(usize, &str)],
) -> Result<Vec<String>, ComputerError> {
    let computers = computers(dir, helper);
    block_on!(async {
        let mut server = JoinSet::new();
        let port = serve(&mut server).await;
        let threads = scripts.iter().map(|(thread, _)| thread + 1).max();
        let mut workspaces = Vec::new();
        for _ in 0..threads.unwrap_or_default() {
            workspaces.push(thread(&computers, dir).await?);
        }
        let mut printed = Vec::new();
        for (thread, script) in scripts {
            let command = sandboxed(dir, network, &format!("SERVER={port}\n{script}"));
            printed.push(said(&computers, workspaces[*thread], &command).await?);
        }
        Ok(printed)
    })
}

/// [`run`], every script in one thread.
fn run_one(
    dir: &Path,
    helper: Option<&Path>,
    network: SandboxNetwork,
    scripts: &[&str],
) -> Result<Vec<String>, ComputerError> {
    let scripts: Vec<(usize, &str)> = scripts.iter().map(|script| (0, *script)).collect();
    run(dir, helper, network, &scripts)
}

/// Starts a server on loopback that answers each request with [`HELLO`], as a task of
/// `server`'s, and says its port.
async fn serve(server: &mut JoinSet<()>) -> u16 {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    server.spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            answer(&mut stream).await;
        }
    });
    port
}

/// Reads a request from `stream` and answers it with [`HELLO`].
async fn answer<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S) {
    let mut read = vec![0_u8; 4096];
    let _ = stream.read(&mut read).await;
    let reply = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{HELLO}",
        HELLO.len()
    );
    let _ = stream.write_all(reply.as_bytes()).await;
}

/// `curl` fetching `url`, printing the status and the body, and what failed if it did.
fn fetch(options: &str, url: &str) -> String {
    format!("curl -sS --max-time 20 -w ' status=%{{http_code}}' {options} \"{url}\" 2>&1; true")
}

/// The stand-in host is reached through the proxy, with the proxy's variables and only those:
/// its secret in their URL and in no other variable, loopback left to the command, and a direct
/// connection to anywhere else fails, as it would for a program that ignores the variables.
/// This computer's loopback is reached directly on macOS, which has no network of a command's
/// own, and not on Linux, where the command's loopback is its thread's namespace's.
#[test]
fn sandbox_proxied_commands_reach_a_host_only_through_the_proxy() {
    let Some(Kept { helper }) = kept() else {
        return;
    };
    let dir = scratch("proxied");
    let said = run_one(
        &dir,
        helper.as_deref(),
        SandboxNetwork::Proxied,
        &[
            &fetch("", "http://allowed.example:$SERVER/"),
            "for name in HTTPS_PROXY all_proxy NO_PROXY; do printenv $name; done; \
             secret=${HTTPS_PROXY#http://t:}; secret=${secret%@*}; \
             echo \"secret in $(env | grep -c -- \"$secret\")\"",
            &fetch("--noproxy '*'", "http://127.0.0.1:$SERVER/"),
            &fetch("--noproxy '*'", "http://93.184.216.34:$SERVER/"),
        ],
    )
    .unwrap();
    let [through, variables, loopback, direct] = said.as_slice() else {
        panic!("{said:?}");
    };
    assert!(through.contains(HELLO), "{through}");
    assert!(through.contains("status=200"), "{through}");
    let mut lines = variables.lines();
    let https = lines.next().unwrap_or_default();
    assert!(
        https.starts_with("http://t:") && https.contains("@127.0.0.1:"),
        "{variables}"
    );
    assert_eq!(lines.next(), Some(https), "{variables}");
    assert_eq!(lines.next(), Some("localhost,127.0.0.1,::1"), "{variables}");
    // In the six variables that name the proxy, and nowhere else.
    assert_eq!(lines.next(), Some("secret in 6"), "{variables}");
    assert!(!direct.contains(HELLO), "{direct}");
    assert!(direct.contains("status=000"), "{direct}");
    if cfg!(target_os = "macos") {
        assert!(loopback.contains(HELLO), "{loopback}");
    } else {
        assert!(!loopback.contains(HELLO), "{loopback}");
        assert!(loopback.contains("status=000"), "{loopback}");
        // Control: with the network and no proxy, this computer's loopback is reached directly.
        let said = run_one(
            &dir,
            helper.as_deref(),
            SandboxNetwork::Direct,
            &[&fetch("", "http://127.0.0.1:$SERVER/")],
        )
        .unwrap();
        assert!(said.concat().contains(HELLO), "{said:?}");
    }
}

/// Cloud metadata and a private network are refused by the proxy, by name or by address, and
/// the model is told why; the proxy turns away a client without its secret, or with another.
#[test]
fn sandbox_proxied_commands_reach_no_metadata_private_address_or_proxy_without_its_secret() {
    let Some(Kept { helper }) = kept() else {
        return;
    };
    let dir = scratch("withheld");
    let bare = r#"--proxy "http://127.0.0.1:${HTTP_PROXY##*:}""#;
    let wrong = r#"--proxy "http://t:wrong@127.0.0.1:${HTTP_PROXY##*:}""#;
    let said = run_one(
        &dir,
        helper.as_deref(),
        SandboxNetwork::Proxied,
        &[
            &fetch("", "http://metadata.example/"),
            &fetch("", "http://169.254.169.254/latest/meta-data/"),
            &fetch("", "http://inside.example/"),
            &fetch("", "http://10.0.0.5/"),
            &fetch(bare, "http://allowed.example:$SERVER/"),
            &fetch(wrong, "http://allowed.example:$SERVER/"),
        ],
    )
    .unwrap();
    let (withheld, without) = said.split_at(4);
    for refused in withheld {
        assert!(refused.contains("status=403"), "{refused}");
        assert!(refused.contains("withheld"), "{refused}");
    }
    for refused in without {
        assert!(refused.contains("status=407"), "{refused}");
        assert!(!refused.contains(HELLO), "{refused}");
    }
}

/// On macOS every sandboxed command reaches every port of loopback, so another thread's proxy
/// too: its secret is what turns this thread's commands away there.
#[cfg(target_os = "macos")]
#[test]
fn sandbox_another_thread_s_proxy_refuses_this_thread_s_secret() {
    if !enforced() {
        return;
    }
    let dir = scratch("threads");
    let theirs = r#"--proxy "${HTTP_PROXY%:*}:$(cat their-port)""#;
    let said = run(
        &dir,
        None,
        SandboxNetwork::Proxied,
        &[
            (
                0,
                r#"printf '%s' "${HTTP_PROXY##*:}" > their-port; echo written"#,
            ),
            (
                1,
                r#"test "${HTTP_PROXY##*:}" != "$(cat their-port)" && echo another"#,
            ),
            (1, &fetch(theirs, "http://allowed.example:$SERVER/")),
            (1, &fetch("", "http://allowed.example:$SERVER/")),
        ],
    )
    .unwrap();
    let [written, another, refused, own] = said.as_slice() else {
        panic!("{said:?}");
    };
    assert!(written.contains("written"), "{written}");
    assert!(another.contains("another"), "{another}");
    assert!(refused.contains("status=407"), "{refused}");
    assert!(!refused.contains(HELLO), "{refused}");
    assert!(own.contains(HELLO), "{own}");
}

/// A named gap: Seatbelt's `localhost` is never `::ffff:127.0.0.1`, so a dual-stack client
/// that reaches the proxy as that address (Java's by default) is refused, while the same
/// client reaching `127.0.0.1` gets through.
#[cfg(target_os = "macos")]
#[test]
fn sandbox_a_dual_stack_client_reaches_the_proxy_only_as_loopback_itself() {
    if !enforced() {
        return;
    }
    let dir = scratch("dual-stack");
    let connect = |to: &str, family: &str| {
        format!(
            "python3 -c \"import os, socket; s = socket.socket(socket.{family}); \
             s.connect(('{to}', int(os.environ['HTTP_PROXY'].rsplit(':', 1)[1]))); print('reached')\" \
             2>&1 | tail -n 1"
        )
    };
    let said = run_one(
        &dir,
        None,
        SandboxNetwork::Proxied,
        &[
            &connect("::ffff:127.0.0.1", "AF_INET6"),
            &connect("127.0.0.1", "AF_INET"),
        ],
    )
    .unwrap();
    let [mapped, plain] = said.as_slice() else {
        panic!("{said:?}");
    };
    assert!(!mapped.contains("reached"), "{mapped}");
    assert!(plain.contains("reached"), "{plain}");
}

/// TLS through the proxy, checked by the system's trust (`curl` on Security.framework, whose
/// checks run in `trustd`): a server whose authority the client is told to trust is reached,
/// and the same server is refused when it is not, so the check really ran in the sandbox.
#[cfg(target_os = "macos")]
#[test]
fn sandbox_tls_through_the_proxy_is_checked_by_the_system_s_trust() {
    if !enforced() {
        return;
    }
    let dir = scratch("tls");
    let (acceptor, authority) = tls_server();
    std::fs::write(dir.join("app/authority.pem"), authority).unwrap();
    let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!(
        "https://allowed.example:{}/",
        listener.local_addr().unwrap().port()
    );
    // Served on a thread of its own, stopped when `stop` is sent or dropped.
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let listener = TcpListener::from_std(listener).unwrap();
            tokio::select! {
                () = serve_tls(listener, acceptor) => {}
                _ = stopped => {}
            }
        });
    });
    let curl =
        |options: &str| format!("CURL_SSL_BACKEND=secure-transport {}", fetch(options, &url));
    let said = run_one(
        &dir,
        None,
        SandboxNetwork::Proxied,
        &[&curl("--cacert authority.pem"), &curl("")],
    )
    .unwrap();
    let _ = stop.send(());
    server.join().unwrap();
    let [trusted, untrusted] = said.as_slice() else {
        panic!("{said:?}");
    };
    assert!(trusted.contains(HELLO), "{trusted}");
    assert!(trusted.contains("status=200"), "{trusted}");
    assert!(!untrusted.contains(HELLO), "{untrusted}");
    assert!(untrusted.contains("status=000"), "{untrusted}");
}

/// An acceptor for `allowed.example`, its certificate signed by an authority of the test's own,
/// and that authority's certificate, as PEM. Valid from this year's start to next year's end,
/// within the 825 days macOS allows a server's certificate.
#[cfg(target_os = "macos")]
fn tls_server() -> (tokio_rustls::TlsAcceptor, String) {
    use glave_ports::Clock as _;
    let now = glave_ports::SystemClock.now().to_string();
    let year: i32 = now.split('-').next().unwrap().parse().unwrap();
    let dated = |names: Vec<String>| {
        let mut params = rcgen::CertificateParams::new(names).unwrap();
        params.not_before = rcgen::date_time_ymd(year, 1, 1);
        params.not_after = rcgen::date_time_ymd(year + 1, 12, 28);
        params
    };
    let authority_key = rcgen::KeyPair::generate().unwrap();
    let mut authority = dated(Vec::new());
    authority.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let authority_pem = authority.self_signed(&authority_key).unwrap().pem();
    let issuer = rcgen::Issuer::from_params(&authority, &authority_key);
    let key = rcgen::KeyPair::generate().unwrap();
    let mut leaf = dated(vec!["allowed.example".into()]);
    leaf.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    let leaf = leaf.signed_by(&key, &issuer).unwrap();
    let config = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![leaf.der().clone()],
        rustls::pki_types::PrivateKeyDer::try_from(key.serialize_der()).unwrap(),
    )
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(config));
    (acceptor, authority_pem)
}

/// Answers each TLS connection `listener` takes with [`HELLO`].
#[cfg(target_os = "macos")]
async fn serve_tls(listener: TcpListener, acceptor: tokio_rustls::TlsAcceptor) {
    while let Ok((stream, _)) = listener.accept().await {
        let Ok(mut stream) = acceptor.accept(stream).await else {
            continue;
        };
        answer(&mut stream).await;
        let _ = stream.shutdown().await;
    }
}

/// Nothing but the proxy's connections leaves the command, so no name is looked up from
/// inside it. On Linux a datagram to a resolver on this computer's loopback never arrives,
/// and one to an address elsewhere has no way out. On macOS a datagram to loopback arrives,
/// since loopback is shared, but one to an address elsewhere is refused, and the system's
/// resolver is out of reach, so no name but `localhost` resolves.
#[test]
fn sandbox_proxied_commands_send_no_datagram_out_so_no_lookup() {
    let Some(Kept { helper }) = kept() else {
        return;
    };
    let dir = scratch("datagrams");
    let resolver = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    resolver
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let port = resolver.local_addr().unwrap().port();
    let send = |to: &str| {
        format!(
            "python3 -c \"import socket; s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); \
             s.sendto(b'lookup', ('{to}', {port})); print('sent')\" 2>&1 | tail -n 1"
        )
    };
    let lookup = |name: &str| {
        format!(
            "python3 -c \"import socket; print(socket.getaddrinfo('{name}', 80)[0][4][0])\" \
             2>/dev/null || echo no-lookup"
        )
    };
    let said = run_one(
        &dir,
        helper.as_deref(),
        SandboxNetwork::Proxied,
        &[
            &send("127.0.0.1"),
            &send("10.0.0.53"),
            &lookup("allowed.example"),
            &lookup("localhost"),
        ],
    )
    .unwrap();
    let [here, elsewhere, name, localhost] = said.as_slice() else {
        panic!("{said:?}");
    };
    assert!(here.contains("sent"), "{here}");
    assert_eq!(name.trim(), "no-lookup", "{name}");
    let mut got = [0_u8; 16];
    if cfg!(target_os = "macos") {
        assert!(elsewhere.contains("not permitted"), "{elsewhere}");
        assert!(
            ["::1", "127.0.0.1"].contains(&localhost.trim()),
            "{localhost}"
        );
        let (n, _) = resolver.recv_from(&mut got).unwrap();
        assert_eq!(got.get(..n), Some(&b"lookup"[..]));
        return;
    }
    assert!(elsewhere.contains("unreachable"), "{elsewhere}");
    assert!(resolver.recv_from(&mut got).is_err(), "{here}");
    // Control: with the network and no proxy, the datagram arrives.
    run_one(
        &dir,
        helper.as_deref(),
        SandboxNetwork::Direct,
        &[&send("127.0.0.1")],
    )
    .unwrap();
    resolver
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let (n, _) = resolver.recv_from(&mut got).unwrap();
    assert_eq!(got.get(..n), Some(&b"lookup"[..]));
}

/// On Linux a thread's commands kept to the proxy share one network namespace: a server one of
/// them keeps running in the background is reached at loopback by the thread's next command,
/// a command let out directly between included, and not by another thread's, in a namespace of
/// its own; once the threads' workspaces close, no process is left in either namespace, so both
/// are gone.
#[cfg(target_os = "linux")]
#[test]
fn sandbox_a_thread_s_proxied_commands_share_one_loopback() {
    let Some(Kept { helper }) = kept() else {
        return;
    };
    let dir = scratch("one-loopback");
    let computers = computers(&dir, helper.as_deref());
    let proxied = |script: &str| sandboxed(&dir, SandboxNetwork::Proxied, script);
    // A server at a port of loopback, which it writes down once it listens.
    let serve = proxied(
        "python3 -c \"import http.server, os; \
         s = http.server.HTTPServer(('127.0.0.1', 0), http.server.SimpleHTTPRequestHandler); \
         open('port.new', 'w').write(str(s.server_port)); os.rename('port.new', 'port'); \
         s.serve_forever()\"",
    );
    let reach = proxied(&format!(
        "for _ in $(seq 100); do test -s port && break; sleep 0.1; done; {}",
        fetch("--noproxy '*'", "http://127.0.0.1:$(cat port)/")
    ));
    let network = proxied("readlink /proc/self/ns/net");
    let let_out = sandboxed(&dir, SandboxNetwork::Direct, "true");
    let (reached, refused, namespaces) = block_on!(async {
        let ours = thread(&computers, &dir).await.unwrap();
        let theirs = thread(&computers, &dir).await.unwrap();
        computers.start(ours, &serve).await.unwrap();
        // A command let out directly between, in a sandbox of its own, leaves the thread's
        // network as it was.
        said(&computers, ours, &reach).await.unwrap();
        said(&computers, ours, &let_out).await.unwrap();
        let reached = said(&computers, ours, &reach).await.unwrap();
        let refused = said(&computers, theirs, &reach).await.unwrap();
        let namespaces = [
            said(&computers, ours, &network).await.unwrap(),
            said(&computers, theirs, &network).await.unwrap(),
        ];
        computers.close(ours).await.unwrap();
        computers.close(theirs).await.unwrap();
        (reached, refused, namespaces)
    });
    assert!(reached.contains("status=200"), "{reached}");
    assert!(refused.contains("status=000"), "{refused}");
    let [ours, theirs] = namespaces.map(|said| said.trim().to_owned());
    assert!(ours.starts_with("net:["), "{ours}");
    assert_ne!(ours, theirs);
    for namespace in [ours, theirs] {
        assert!(gone(&namespace), "{namespace} is still in use");
    }
}

/// On Linux a thread whose forwarder was stopped from outside glave has lost its network, and
/// its next command kept to the proxy starts in a new one, never in this computer's.
#[cfg(target_os = "linux")]
#[test]
fn sandbox_a_thread_whose_forwarder_was_stopped_gets_a_new_network() {
    let Some(Kept { helper }) = kept() else {
        return;
    };
    let dir = scratch("forwarder-stopped");
    let computers = computers(&dir, helper.as_deref());
    let network = sandboxed(&dir, SandboxNetwork::Proxied, "readlink /proc/self/ns/net");
    let (first, second) = block_on!(async {
        let ours = thread(&computers, &dir).await.unwrap();
        let first = said(&computers, ours, &network).await.unwrap();
        // With no command running, the forwarder alone is in it.
        for pid in in_namespace(first.trim()) {
            let pid = rustix::process::Pid::from_raw(pid).unwrap();
            rustix::process::kill_process(pid, rustix::process::Signal::KILL).unwrap();
        }
        assert!(gone(first.trim()), "{first}");
        let second = said(&computers, ours, &network).await.unwrap();
        (first, second)
    });
    let here = std::fs::read_link("/proc/self/ns/net").unwrap();
    assert!(second.starts_with("net:["), "{second}");
    assert_ne!(first.trim(), second.trim());
    assert_ne!(Path::new(second.trim()), here);
}

/// The processes in `namespace` (`net:[<inode>]`) that this one may look at.
#[cfg(target_os = "linux")]
fn in_namespace(namespace: &str) -> Vec<i32> {
    std::fs::read_dir("/proc")
        .unwrap()
        .flatten()
        .filter(|process| {
            std::fs::read_link(process.path().join("ns/net"))
                .is_ok_and(|link| link.as_os_str() == namespace)
        })
        .filter_map(|process| process.file_name().to_str()?.parse().ok())
        .collect()
}

/// Whether no process is in `namespace` any more, within seconds: what was stopped takes a
/// moment to end.
#[cfg(target_os = "linux")]
fn gone(namespace: &str) -> bool {
    let until = std::time::Instant::now() + Duration::from_secs(10);
    while !in_namespace(namespace).is_empty() {
        if std::time::Instant::now() > until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    true
}
