//! A tiny web server on 127.0.0.1 for tests and evals (the `pages` feature): fixed replies by
//! path, so the web tools can be exercised with no network. HTTP/1.1, one request per
//! connection. Not for serving anything real.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};

/// The most bytes of a request head read.
const MOST_HEAD: usize = 16 * 1024;

/// What the server answers at one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// A body with a status and content type.
    Page {
        status: u16,
        content_type: String,
        body: Vec<u8>,
    },
    /// A 302 to this location.
    Redirect(String),
    /// Never answers, holding the connection open.
    Hang,
}

impl Reply {
    /// A 200 HTML page.
    pub fn html(body: &str) -> Self {
        Self::page("text/html; charset=utf-8", body.as_bytes().to_vec())
    }

    /// A 200 reply of `content_type`.
    pub fn page(content_type: &str, body: Vec<u8>) -> Self {
        Self::Page {
            status: 200,
            content_type: content_type.to_owned(),
            body,
        }
    }
}

type Shared = Arc<(HashMap<String, Reply>, Mutex<Vec<String>>, AtomicUsize)>;

/// A running server. It stops when dropped.
pub struct Pages {
    addr: SocketAddr,
    shared: Shared,
    task: JoinHandle<()>,
}

impl Pages {
    /// Serves `replies`, by path (`/a.html`, any query ignored), on a free port of 127.0.0.1.
    /// Other paths are 404.
    pub async fn serve(replies: HashMap<String, Reply>) -> std::io::Result<Self> {
        Self::serve_with(|_| replies).await
    }

    /// Like [`Pages::serve`], with replies made once the server's address (`base`) is known,
    /// for pages that link to each other.
    pub async fn serve_with(
        replies: impl FnOnce(&str) -> HashMap<String, Reply>,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let replies = replies(&format!("http://{addr}"));
        let shared: Shared = Arc::new((replies, Mutex::new(Vec::new()), AtomicUsize::new(0)));
        let serving = Arc::clone(&shared);
        let task = tokio::spawn(async move {
            // Connections live in this set, so stopping the server stops them too.
            let mut connections = JoinSet::new();
            while let Ok((stream, _)) = listener.accept().await {
                connections.spawn(answer(stream, Arc::clone(&serving)));
                while connections.try_join_next().is_some() {}
            }
        });
        Ok(Self { addr, shared, task })
    }

    /// The server's address, such as `http://127.0.0.1:53817`.
    pub fn base(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// The paths asked for so far, with their queries, in order.
    pub fn requested(&self) -> Vec<String> {
        self.shared
            .1
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// How many requests have come in.
    pub fn hits(&self) -> usize {
        self.shared.2.load(Ordering::SeqCst)
    }
}

impl Drop for Pages {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn answer(mut stream: TcpStream, shared: Shared) {
    let mut head = Vec::new();
    let mut buffer = [0_u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < MOST_HEAD {
        match stream.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(n) => head.extend_from_slice(buffer.get(..n).unwrap_or_default()),
        }
    }
    let text = String::from_utf8_lossy(&head);
    let path = text
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_owned();
    shared.2.fetch_add(1, Ordering::SeqCst);
    shared
        .1
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(path.clone());
    let route = path.split('?').next().unwrap_or_default();
    let response = match shared.0.get(route) {
        Some(Reply::Hang) => {
            std::future::pending::<()>().await;
            return;
        }
        Some(Reply::Redirect(to)) => respond(302, "text/plain", Some(to), b""),
        Some(Reply::Page {
            status,
            content_type,
            body,
        }) => respond(*status, content_type, None, body),
        None => respond(404, "text/plain", None, b"not found"),
    };
    let _ = stream.write_all(&response).await;
    let _ = stream.shutdown().await;
}

fn respond(status: u16, content_type: &str, location: Option<&str>, body: &[u8]) -> Vec<u8> {
    let location = location
        .map(|location| format!("location: {location}\r\n"))
        .unwrap_or_default();
    let head = format!(
        "HTTP/1.1 {status} X\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\n\
         connection: close\r\n{location}\r\n",
        body.len()
    );
    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}
