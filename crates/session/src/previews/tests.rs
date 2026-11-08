//! A preview carries a page and a WebSocket to the port it previews, unchanged, listens on
//! loopback alone, closes when its command no longer listens there, and is shown only where a
//! browser of the caller's reaches it.

use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

use glave_event::ThreadId;
use glave_ports::block_on;
use glave_ports::computer::CommandId;
use glave_ports::host::PublicOrigin;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{TcpListener, TcpStream};

use super::*;

/// RFC 6455's own example: a client's key and the accept a server answers it with.
const KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";
const ACCEPT: &str = "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=";

/// How long a step over loopback may take.
const SOON: Duration = Duration::from_secs(10);

fn thread() -> ThreadId {
    ThreadId::new(jiff::Timestamp::UNIX_EPOCH, [1; 10]).unwrap()
}

fn command(n: u128) -> CommandId {
    CommandId::from_uuid(uuid::Uuid::from_u128(0x0190_0000_0000_7000_8000_0000_0000_0100 + n))
}

/// An app on loopback, as a dev server is: a page to any request, saying the `Host` it was
/// asked at, and, asked to upgrade, a WebSocket that echoes one text frame. Answers its port.
async fn app() -> (u16, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let serving = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            tokio::spawn(answer(stream));
        }
    });
    (port, serving)
}

async fn answer(stream: TcpStream) {
    let mut stream = BufReader::new(stream);
    let mut head = Vec::new();
    loop {
        let mut line = String::new();
        stream.read_line(&mut line).await.unwrap();
        if line == "\r\n" || line.is_empty() {
            break;
        }
        head.push(line.trim_end().to_owned());
    }
    let header = |name: &str| {
        head.iter().find_map(|line| {
            let (key, value) = line.split_once(": ")?;
            key.eq_ignore_ascii_case(name).then(|| value.to_owned())
        })
    };
    if header("Upgrade").as_deref() == Some("websocket") {
        assert_eq!(header("Sec-WebSocket-Key").as_deref(), Some(KEY));
        let upgraded = format!(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Accept: {ACCEPT}\r\n\r\n"
        );
        stream.write_all(upgraded.as_bytes()).await.unwrap();
        // One masked text frame of fewer than 126 bytes in, the same text unmasked out.
        let mut first = [0_u8; 2];
        stream.read_exact(&mut first).await.unwrap();
        let length = usize::from(first[1] & 0x7f);
        let mut mask = [0_u8; 4];
        stream.read_exact(&mut mask).await.unwrap();
        let mut text = vec![0_u8; length];
        stream.read_exact(&mut text).await.unwrap();
        for (at, byte) in text.iter_mut().enumerate() {
            *byte ^= mask[at % 4];
        }
        let mut echo = vec![0x81, u8::try_from(length).unwrap()];
        echo.extend(text);
        stream.write_all(&echo).await.unwrap();
    } else {
        let body = format!("served at {}", header("Host").unwrap_or_default());
        let page = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(page.as_bytes()).await.unwrap();
    }
}

/// The port of a preview's address, `http://127.0.0.1:<port>/`.
fn port_of(preview: &Preview) -> u16 {
    let at = preview.url.strip_prefix("http://127.0.0.1:").unwrap();
    at.strip_suffix('/').unwrap().parse().unwrap()
}

async fn connect(port: u16) -> std::io::Result<TcpStream> {
    tokio::time::timeout(SOON, TcpStream::connect(("127.0.0.1", port)))
        .await
        .unwrap()
}

#[test]
fn a_preview_carries_a_page_and_a_websocket_to_the_command_s_port_unchanged() {
    block_on!(async {
        let (port, serving) = app().await;
        let previews = Previews::default();
        let shown = previews.settle(thread(), &[(command(1), port)]).await;
        assert_eq!(shown.len(), 1, "{shown:?}");
        assert_eq!((shown[0].command, shown[0].port), (command(1), port));
        let at = port_of(&shown[0]);
        assert_ne!(at, port, "a port of its own, so an origin of its own");

        // A page, asked at the preview's own address: the app is asked at it too.
        let mut page = connect(at).await.unwrap();
        let asked = format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{at}\r\n\r\n");
        page.write_all(asked.as_bytes()).await.unwrap();
        let mut answered = String::new();
        tokio::time::timeout(SOON, page.read_to_string(&mut answered))
            .await
            .unwrap()
            .unwrap();
        assert!(answered.starts_with("HTTP/1.1 200 OK"), "{answered}");
        assert!(answered.ends_with(&format!("served at 127.0.0.1:{at}")), "{answered}");

        // A WebSocket, as live reload opens one: the upgrade, then a frame each way.
        let mut socket = connect(at).await.unwrap();
        let upgrade = format!(
            "GET /live HTTP/1.1\r\nHost: 127.0.0.1:{at}\r\nUpgrade: websocket\r\n\
             Connection: Upgrade\r\nSec-WebSocket-Key: {KEY}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        socket.write_all(upgrade.as_bytes()).await.unwrap();
        let mut socket = BufReader::new(socket);
        let mut status = String::new();
        socket.read_line(&mut status).await.unwrap();
        assert_eq!(status, "HTTP/1.1 101 Switching Protocols\r\n");
        let mut accepted = false;
        loop {
            let mut line = String::new();
            socket.read_line(&mut line).await.unwrap();
            accepted |= line == format!("Sec-WebSocket-Accept: {ACCEPT}\r\n");
            if line == "\r\n" {
                break;
            }
        }
        assert!(accepted, "the app's own accept comes back");
        let text = b"reload";
        let mask = [1_u8, 2, 3, 4];
        let mut frame = vec![0x81, 0x80 | u8::try_from(text.len()).unwrap()];
        frame.extend(mask);
        frame.extend(text.iter().enumerate().map(|(at, byte)| byte ^ mask[at % 4]));
        socket.write_all(&frame).await.unwrap();
        let mut echo = [0_u8; 8];
        tokio::time::timeout(SOON, socket.read_exact(&mut echo))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&echo[..2], [0x81, 6]);
        assert_eq!(&echo[2..], text);

        previews.close_all().done().await;
        serving.abort();
    });
}

#[test]
fn a_preview_listens_on_loopback_alone_and_closes_once_its_command_no_longer_listens() {
    block_on!(async {
        let (port, serving) = app().await;
        let previews = Previews::default();
        let listening = [(command(1), port), (command(2), port)];
        let shown = previews.settle(thread(), &listening).await;
        assert_eq!(shown.len(), 2, "one for each command");
        let (first, second) = (port_of(&shown[0]), port_of(&shown[1]));
        assert_eq!(previews.settle(thread(), &listening).await, shown, "the same ones");

        // This computer's address toward elsewhere, found without sending anything: nothing
        // there reaches the preview.
        let outward = UdpSocket::bind("0.0.0.0:0")
            .and_then(|socket| {
                socket.connect("192.0.2.1:9")?;
                socket.local_addr()
            })
            .map(|address| address.ip())
            .ok()
            .filter(|ip| !ip.is_loopback() && !ip.is_unspecified());
        if let Some(outward) = outward {
            let there = SocketAddr::new(outward, first);
            let reached = tokio::time::timeout(Duration::from_secs(2), TcpStream::connect(there));
            assert!(
                !matches!(reached.await, Ok(Ok(_))),
                "a preview is not reached at {outward}"
            );
        }

        // The first command no longer listens: its preview closes, the second's stays.
        let left = previews.settle(thread(), &[(command(2), port)]).await;
        assert_eq!(left, shown[1..]);
        assert!(connect(first).await.is_err(), "a closed preview takes nothing");
        assert!(connect(second).await.is_ok());
        // It no longer runs.
        previews
            .keep_running(thread(), |running| running != command(2))
            .done()
            .await;
        assert!(connect(second).await.is_err());
        serving.abort();
    });
}

#[test]
fn previews_reach_a_browser_of_the_person_on_this_computer_alone() {
    let public = PublicOrigin::parse("https://glave.example.com").unwrap();
    let shared = PublicOrigin::parse("http://localhost:4180").unwrap();
    // Served here alone, or at this computer's own localhost: theirs, and only theirs.
    assert!(reached_here(None, false, true));
    assert!(reached_here(Some(&shared), false, true));
    assert!(!reached_here(None, false, false), "not someone else's");
    assert!(!reached_here(Some(&shared), false, false));
    // A browser elsewhere, or a computer of their own elsewhere: not on this loopback.
    assert!(!reached_here(Some(&public), false, true));
    assert!(!reached_here(None, true, true));
}
