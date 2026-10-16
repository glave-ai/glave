//! A fake git host for tests of credentials: one repository served over git's dumb HTTP
//! protocol, only to a client that signs in with the right user name and token (Basic, as git
//! sends a credential helper's answer). It counts the requests it refused. And credentials that
//! answer every host they name with one credential.

use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::computer::bytes::encode;
use crate::computer::{CredentialAsk, CredentialHost, Credentials, GitCredential};
use crate::tools::BoxFuture;

/// The commit the repository's one branch points at.
pub const FAKE_COMMIT: &str = "8d7e5f2a1b3c4d5e6f708192a3b4c5d6e7f80912";

/// A fake git host on 127.0.0.1, serving on a thread of its own until dropped.
#[derive(Debug)]
pub struct FakeGitHost {
    /// `http://` and the address: the repository is `<url>/repo.git`.
    pub url: String,
    stop: Arc<AtomicBool>,
    refused: Arc<AtomicUsize>,
    thread: Option<JoinHandle<()>>,
}

impl FakeGitHost {
    /// Serves the repository to a client signed in as `username` with `token`.
    pub fn start(username: &str, token: &str) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let url = format!("http://{}", listener.local_addr()?);
        let expected = format!("Basic {}", encode(format!("{username}:{token}").as_bytes()));
        let (stop, refused) = (Arc::new(AtomicBool::new(false)), Arc::default());
        let (stopping, counting) = (Arc::clone(&stop), Arc::clone(&refused));
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stopping.load(Ordering::SeqCst) {
                    return;
                }
                if let Ok(stream) = stream {
                    let _ = answer(stream, &expected, &counting);
                }
            }
        });
        Ok(Self {
            url,
            stop,
            refused,
            thread: Some(thread),
        })
    }

    /// How many requests came without the right sign-in.
    pub fn refused(&self) -> usize {
        self.refused.load(Ordering::SeqCst)
    }
}

impl Drop for FakeGitHost {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wakes the thread blocked in accept, so it sees the stop.
        let _ = TcpStream::connect(self.url.trim_start_matches("http://"));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Answers one request: the refs or `HEAD` to a client signed in as `expected`, 401 to anyone
/// else, and 404 for anything it does not serve.
fn answer(stream: TcpStream, expected: &str, refused: &AtomicUsize) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let path = line
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    let mut signed_in = None;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("authorization")
        {
            signed_in = Some(value.trim().to_owned());
        }
    }
    let mut stream = stream;
    let body = if path.starts_with("/repo.git/info/refs") {
        format!("{FAKE_COMMIT}\trefs/heads/main\n")
    } else if path == "/repo.git/HEAD" {
        "ref: refs/heads/main\n".to_owned()
    } else {
        return write!(
            stream,
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
    };
    if signed_in.as_deref() != Some(expected) {
        refused.fetch_add(1, Ordering::SeqCst);
        return write!(
            stream,
            "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"glave\"\r\n\
             Content-Length: 0\r\nConnection: close\r\n\r\n"
        );
    }
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    )
}

/// Credentials for tests: one credential for every host they name, and what they were asked.
#[derive(Debug)]
pub struct FixedCredentials {
    hosts: Vec<CredentialHost>,
    credential: GitCredential,
    asked: Mutex<Vec<CredentialAsk>>,
}

impl FixedCredentials {
    pub fn new(hosts: Vec<CredentialHost>, credential: GitCredential) -> Self {
        Self {
            hosts,
            credential,
            asked: Mutex::new(Vec::new()),
        }
    }

    /// Every request so far, oldest first.
    pub fn asked(&self) -> Vec<CredentialAsk> {
        self.asked
            .lock()
            .map(|asked| asked.clone())
            .unwrap_or_default()
    }
}

impl Credentials for FixedCredentials {
    fn hosts(&self) -> BoxFuture<'_, Vec<CredentialHost>> {
        Box::pin(std::future::ready(self.hosts.clone()))
    }

    fn request<'a>(&'a self, ask: &'a CredentialAsk) -> BoxFuture<'a, Option<GitCredential>> {
        if let Ok(mut asked) = self.asked.lock() {
            asked.push(ask.clone());
        }
        Box::pin(std::future::ready(Some(self.credential.clone())))
    }
}
