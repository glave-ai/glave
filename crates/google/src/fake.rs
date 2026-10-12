//! A fake Google for tests, on 127.0.0.1: it answers each request with the next of its
//! answers, and records what it was asked.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// A request the fake received.
pub(crate) struct Received {
    pub method: String,
    pub path: String,
    /// Header names in lower case.
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

impl Received {
    /// The body as a form.
    pub(crate) fn form(&self) -> BTreeMap<String, String> {
        url::form_urlencoded::parse(self.body.as_bytes())
            .into_owned()
            .collect()
    }
}

pub(crate) struct FakeGoogle {
    pub url: String,
    /// The address alone, as `GCE_METADATA_HOST` takes it.
    pub host: String,
    received: Arc<Mutex<Vec<Received>>>,
    thread: Option<JoinHandle<()>>,
}

impl FakeGoogle {
    pub(crate) fn start(answers: Vec<(u16, String)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let host = listener.local_addr().unwrap().to_string();
        let received = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&received);
        let thread = std::thread::spawn(move || {
            for (status, body) in answers {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut words = line.split_whitespace();
                let method = words.next().unwrap().to_owned();
                let path = words.next().unwrap().to_owned();
                let mut headers = BTreeMap::new();
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
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
                    .map_or(0, |length| length.parse().unwrap());
                let mut sent = vec![0; length];
                reader.read_exact(&mut sent).unwrap();
                log.lock().unwrap().push(Received {
                    method,
                    path,
                    headers,
                    body: String::from_utf8(sent).unwrap(),
                });
                let mut stream = stream;
                write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: \
                     {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        Self {
            url: format!("http://{host}"),
            host,
            received,
            thread: Some(thread),
        }
    }

    /// Every request, once all the answers have been given.
    pub(crate) fn received(mut self) -> Vec<Received> {
        self.thread.take().unwrap().join().unwrap();
        std::mem::take(&mut *self.received.lock().unwrap())
    }
}
