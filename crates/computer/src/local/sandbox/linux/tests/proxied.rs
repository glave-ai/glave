//! Commands kept to the outbound proxy, in the real sandbox: each in a network namespace of
//! its own whose one way out is glave's forwarder to the proxy. The proxy reaches a stand-in
//! for the internet (names resolve to what these tests say, and the one public address is a
//! server on this computer), so nothing leaves it. They need the built glave as the forwarder
//! (`GLAVE_TEST_SANDBOX_HELPER`), and must find it where `GLAVE_REQUIRE_SANDBOX` is set.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::time::Duration;

use glave_ports::block_on;
use glave_ports::computer::{
    Command, ComputerError, Computers, Ending, Environment, Sandbox, SandboxLevel, Sources,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use super::{Collect, bubblewrap, scratch};
use crate::local::sandbox::Outbound;
use crate::proxy::{Mode, Network, Rules};
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

/// The built glave, which forwards in each command's namespace, where these tests run.
fn forwarder() -> Option<PathBuf> {
    if !bubblewrap() {
        return None;
    }
    let helper = std::env::var_os("GLAVE_TEST_SANDBOX_HELPER").map(PathBuf::from);
    assert!(
        helper.is_some() || std::env::var_os("GLAVE_REQUIRE_SANDBOX").is_none(),
        "GLAVE_REQUIRE_SANDBOX is set but GLAVE_TEST_SANDBOX_HELPER, the built glave, is not"
    );
    helper
}

/// Runs each of `scripts` in `dir/project`, in the `project` sandbox with the network, kept to
/// the proxy or not, while an HTTP server answers [`HELLO`] on loopback at the port each
/// script's `$SERVER` names. Returns what each printed.
fn run(
    dir: &Path,
    helper: &Path,
    proxied: bool,
    scripts: &[&str],
) -> Result<Vec<String>, ComputerError> {
    let environment: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(name, _)| name == "PATH")
        .chain([("HOME".into(), dir.as_os_str().to_owned())])
        .collect();
    let computers = LocalComputers::new(dir.join("copies"), environment)
        .with_helper(helper)
        .with_outbound(Outbound::over(Rules::new(Mode::Open, []), StandIn::new()));
    let sandbox = Sandbox {
        level: SandboxLevel::Project,
        network: true,
        proxied,
        home: Some(dir.to_owned()),
        glave_home: Some(dir.join(".glave")),
    };
    block_on!(async {
        let mut server = JoinSet::new();
        let port = serve(&mut server).await;
        let computer = computers.create(&Environment::host()).await?;
        let sources = Sources::Folder {
            path: dir.join("project").to_string_lossy().into_owned(),
        };
        let workspace = computers.open(computer.id, &sources).await?.id;
        let mut said = Vec::new();
        for script in scripts {
            let mut command = Command::new(format!("SERVER={port}\n{script}"));
            command.sandbox = Some(sandbox.clone());
            command.timeout_secs = Some(60);
            let output = Collect::default();
            let finished = computers
                .run(workspace, &command, &output, &CancelSwitch::default())
                .await?;
            let printed = output.0.lock().unwrap().clone();
            assert!(
                !matches!(finished.ending, Ending::TimedOut { .. }),
                "{script}: {printed}"
            );
            said.push(printed);
        }
        Ok(said)
    })
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
            let mut read = vec![0_u8; 4096];
            let _ = stream.read(&mut read).await;
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{HELLO}",
                HELLO.len()
            );
            let _ = stream.write_all(reply.as_bytes()).await;
        }
    });
    port
}

/// `curl` fetching `url`, printing the status and the body, and what failed if it did.
fn fetch(options: &str, url: &str) -> String {
    format!("curl -sS --max-time 20 -w ' status=%{{http_code}}' {options} \"{url}\" 2>&1; true")
}

/// The stand-in host is reached through the proxy, with the variables the forwarder sets and
/// only those: no secret in the command's own variables, loopback left to the command, and a
/// direct connection to anywhere, this computer's loopback included, fails, as it would for a
/// program that ignores the variables.
#[test]
fn sandbox_proxied_commands_reach_a_host_only_through_the_proxy() {
    let Some(helper) = forwarder() else {
        return;
    };
    let dir = scratch("proxied");
    let said = run(
        &dir,
        &helper,
        true,
        &[
            &fetch("", "http://allowed.example:$SERVER/"),
            "printenv HTTPS_PROXY all_proxy NO_PROXY; printenv GLAVE_PROXY_SECRET || echo no-secret",
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
    assert_eq!(lines.next(), Some("no-secret"), "{variables}");
    for direct in [loopback, direct] {
        assert!(!direct.contains(HELLO), "{direct}");
        assert!(direct.contains("status=000"), "{direct}");
    }
    // Control: with the network and no proxy, this computer's loopback is reached directly.
    let said = run(
        &dir,
        &helper,
        false,
        &[&fetch("", "http://127.0.0.1:$SERVER/")],
    )
    .unwrap();
    assert!(said.concat().contains(HELLO), "{said:?}");
}

/// Cloud metadata and a private network are refused by the proxy, by name or by address, and
/// the model is told why; the proxy turns away a client without its secret, or with another.
#[test]
fn sandbox_proxied_commands_reach_no_metadata_private_address_or_proxy_without_its_secret() {
    let Some(helper) = forwarder() else {
        return;
    };
    let dir = scratch("withheld");
    let bare = r#"--proxy "http://127.0.0.1:${HTTP_PROXY##*:}""#;
    let wrong = r#"--proxy "http://t:wrong@127.0.0.1:${HTTP_PROXY##*:}""#;
    let said = run(
        &dir,
        &helper,
        true,
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

/// Nothing but the proxy's connections leaves the namespace, so no name is looked up from
/// inside it: a datagram to a resolver on this computer's loopback never arrives, and one to an
/// address elsewhere has no way out.
#[test]
fn sandbox_proxied_commands_send_no_datagram_so_no_lookup() {
    let Some(helper) = forwarder() else {
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
    let said = run(
        &dir,
        &helper,
        true,
        &[&send("127.0.0.1"), &send("10.0.0.53")],
    )
    .unwrap();
    let [here, elsewhere] = said.as_slice() else {
        panic!("{said:?}");
    };
    assert!(here.contains("sent"), "{here}");
    let mut got = [0_u8; 16];
    assert!(resolver.recv_from(&mut got).is_err(), "{here}");
    assert!(elsewhere.contains("unreachable"), "{elsewhere}");
    // Control: with the network and no proxy, the datagram arrives.
    run(&dir, &helper, false, &[&send("127.0.0.1")]).unwrap();
    resolver
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let (n, _) = resolver.recv_from(&mut got).unwrap();
    assert_eq!(got.get(..n), Some(&b"lookup"[..]));
}
