//! One preview's listener: each connection a browser opens to it carried to the port its
//! command listens on, on this computer's loopback, bytes as they are both ways, so an HTTP
//! request, its answer and a WebSocket opened by one all go through unchanged, the `Host` a
//! browser sent among them. Bounded: so many connections at once, a few seconds to reach the
//! command's port, and an hour carrying nothing either way.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

/// The most connections one preview carries at once: a browser opens six to an origin, and a
/// page's WebSockets beside them. One more is closed at once.
const MOST: usize = 64;
/// How long reaching the command's port may take: a program on this computer answers at once.
const REACH: Duration = Duration::from_secs(5);
/// How long a connection may carry nothing either way before it is closed, as a live-reload
/// socket left in a tab no one looks at; the page opens another when it is looked at again.
const QUIET: Duration = Duration::from_hours(1);
/// What is read from either side at once.
const CHUNK: usize = 16 * 1024;
/// How long to wait before taking connections again when the system refused one (too many
/// files open, say), so a refusal that lasts is no busy loop.
const REFUSED: Duration = Duration::from_millis(100);

/// Carries each connection to `listener` to `port` on this computer's loopback, until it is
/// dropped, which stops every connection it carries with it.
#[expect(
    clippy::infinite_loop,
    reason = "it runs until its preview drops it, and an async fn cannot return `!`"
)]
pub(super) async fn carry(listener: TcpListener, port: u16) {
    let mut carried = JoinSet::new();
    loop {
        while carried.try_join_next().is_some() {}
        let Ok((browser, _)) = listener.accept().await else {
            tokio::time::sleep(REFUSED).await;
            continue;
        };
        // Past the most, it closes as it drops.
        if carried.len() < MOST {
            carried.spawn(through(browser, port));
        }
    }
}

/// Carries `browser`'s connection to `port`, until either side closes it or it is quiet.
async fn through(browser: TcpStream, port: u16) {
    if let Some(app) = reached(port).await {
        pump(browser, app).await;
    }
}

/// A connection to `port` on loopback: IPv4's first, then IPv6's, where a server bound to
/// `localhost` may listen alone.
async fn reached(port: u16) -> Option<TcpStream> {
    for address in [Ipv4Addr::LOCALHOST.into(), Ipv6Addr::LOCALHOST.into()] {
        let connecting = TcpStream::connect(std::net::SocketAddr::new(address, port));
        if let Ok(Ok(stream)) = tokio::time::timeout(REACH, connecting).await {
            return Some(stream);
        }
    }
    None
}

/// Which side a read came from.
enum Read {
    Browser(std::io::Result<usize>),
    App(std::io::Result<usize>),
}

/// Copies bytes both ways between `browser` and `app` until both have finished sending, either
/// fails, or nothing moves either way for [`QUIET`]. One side finishing sending is passed on
/// to the other (its write half shut), which may still answer.
async fn pump(browser: TcpStream, app: TcpStream) {
    let (mut from_browser, mut to_browser) = browser.into_split();
    let (mut from_app, mut to_app) = app.into_split();
    let (mut up, mut down) = (vec![0_u8; CHUNK], vec![0_u8; CHUNK]);
    let (mut browser_sends, mut app_sends) = (true, true);
    while browser_sends || app_sends {
        let read = tokio::time::timeout(QUIET, async {
            tokio::select! {
                read = from_browser.read(&mut up), if browser_sends => Read::Browser(read),
                read = from_app.read(&mut down), if app_sends => Read::App(read),
            }
        })
        .await;
        let moved = match read {
            Err(_) => return,
            Ok(Read::Browser(Ok(0))) => {
                browser_sends = false;
                to_app.shutdown().await
            }
            Ok(Read::App(Ok(0))) => {
                app_sends = false;
                to_browser.shutdown().await
            }
            Ok(Read::Browser(Ok(read))) => {
                let bytes = up.get(..read).unwrap_or_default();
                written(to_app.write_all(bytes)).await
            }
            Ok(Read::App(Ok(read))) => {
                let bytes = down.get(..read).unwrap_or_default();
                written(to_browser.write_all(bytes)).await
            }
            Ok(Read::Browser(Err(error)) | Read::App(Err(error))) => Err(error),
        };
        if moved.is_err() {
            return;
        }
    }
}

/// A write, within [`QUIET`]: a side that takes nothing for that long is gone.
async fn written(
    write: impl Future<Output = std::io::Result<()>>,
) -> std::io::Result<()> {
    tokio::time::timeout(QUIET, write)
        .await
        .unwrap_or_else(|_| Err(std::io::ErrorKind::TimedOut.into()))
}
