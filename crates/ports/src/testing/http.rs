//! A fake HTTP server for tests of clients that call a cloud: it answers each request with the
//! next of its answers, one connection each, and records what it was asked.

use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

/// A request the fake received.
#[derive(Debug)]
pub struct Received {
    pub method: String,
    pub path: String,
    /// Header names in lower case.
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

/// A fake server on 127.0.0.1, serving its answers on a thread of its own.
#[derive(Debug)]
pub struct FakeHttp {
    /// `http://` and the address.
    pub url: String,
    /// The address alone.
    pub host: String,
    received: Arc<Mutex<Vec<Received>>>,
    thread: Option<JoinHandle<()>>,
}

impl FakeHttp {
    /// Starts serving `answers`, each a status and a JSON body, one per request in order.
    pub fn start(answers: Vec<(u16, String)>) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let host = listener.local_addr()?.to_string();
        let received = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&received);
        let thread = std::thread::spawn(move || {
            for (status, body) in answers {
                // A client that goes away ends the serving; the test sees fewer requests.
                let Ok(request) = answer(&listener, status, &body) else {
                    return;
                };
                log.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(request);
            }
        });
        Ok(Self {
            url: format!("http://{host}"),
            host,
            received,
            thread: Some(thread),
        })
    }

    /// Every request, once all the answers have been given.
    pub fn received(mut self) -> Vec<Received> {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        std::mem::take(&mut *self.received.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// Takes the next connection, reads its request and answers `status` with `body`.
fn answer(listener: &TcpListener, status: u16, body: &str) -> io::Result<Received> {
    let (mut stream, _) = listener.accept()?;
    let request = read(&stream)?;
    write!(
        stream,
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    )?;
    Ok(request)
}

fn read(stream: &TcpStream) -> io::Result<Received> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut words = line.split_whitespace();
    let method = words.next().unwrap_or_default().to_owned();
    let path = words.next().unwrap_or_default().to_owned();
    let mut headers = BTreeMap::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let length = headers
        .get("content-length")
        .and_then(|length| length.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Received {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}
