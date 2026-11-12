//! A fake HTTP server for tests of clients that call a cloud: it answers each request with the
//! next of its answers, or with the same one until it is done, one connection each, and records
//! what it was asked.

use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

/// A request the fake received.
#[derive(Debug, Clone)]
pub struct Received {
    pub method: String,
    pub path: String,
    /// Header names in lower case.
    pub headers: BTreeMap<String, String>,
    pub body: String,
    /// The body as it was sent, for one that is not text, such as an encrypted push.
    pub bytes: Vec<u8>,
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
    /// Set to stop one that answers until it is done.
    stop: Option<Arc<AtomicBool>>,
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
            stop: None,
        })
    }

    /// Starts answering every request with `status` and the JSON `body`, until its requests are
    /// asked for or it is dropped.
    pub fn always(status: u16, body: &str) -> io::Result<Self> {
        let body = body.to_owned();
        Self::answering(move |_, _| (status, body.clone()))
    }

    /// Starts answering each request with the status and JSON body `answer` gives for its
    /// method and path, as a stand-in for a whole API, until its requests are asked for or it
    /// is dropped. A redirect's body is where it goes.
    pub fn answering(
        answer: impl Fn(&str, &str) -> (u16, String) + Send + 'static,
    ) -> io::Result<Self> {
        Self::answering_requests(move |request| answer(&request.method, &request.path))
    }

    /// Starts answering each request with the status and JSON body `answer` gives for all of it,
    /// its body too, as a stand-in that reads what it is sent, until its requests are asked for
    /// or it is dropped.
    pub fn answering_requests(
        answer: impl Fn(&Received) -> (u16, String) + Send + 'static,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let host = listener.local_addr()?.to_string();
        let received = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (log, stopping) = (Arc::clone(&received), Arc::clone(&stop));
        let thread = std::thread::spawn(move || serve(&listener, &answer, &log, &stopping));
        Ok(Self {
            url: format!("http://{host}"),
            host,
            received,
            thread: Some(thread),
            stop: Some(stop),
        })
    }

    /// Every request, once all the answers have been given, or one that answers until it is
    /// done has stopped.
    pub fn received(mut self) -> Vec<Received> {
        self.finish();
        std::mem::take(&mut *self.received.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Waits for its answers to end, stopping one that answers until it is done.
    fn finish(&mut self) {
        if let Some(stop) = &self.stop {
            stop.store(true, Ordering::SeqCst);
            // Wakes the thread blocked in accept, so it sees the stop.
            let _ = TcpStream::connect(&self.host);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for FakeHttp {
    fn drop(&mut self) {
        if self.stop.is_some() {
            self.finish();
        }
    }
}

/// Answers each connection `listener` takes with what `answer` gives, keeping its request in
/// `log`, until `stop` is set. The stop is read as a connection is taken, not once it is
/// answered: the connection taken after it is the one that wakes the thread, and a request taken
/// before it, whose answer its client may already have, is kept however late it is answered.
fn serve(
    listener: &TcpListener,
    answer: &dyn Fn(&Received) -> (u16, String),
    log: &Mutex<Vec<Received>>,
    stop: &AtomicBool,
) {
    for stream in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        if let Ok(request) = stream.and_then(|stream| answer_with(stream, answer)) {
            log.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(request);
        }
    }
}

/// Takes the next connection, reads its request and answers `status` with `body`.
fn answer(listener: &TcpListener, status: u16, body: &str) -> io::Result<Received> {
    let (stream, _) = listener.accept()?;
    answer_with(stream, &|_| (status, body.to_owned()))
}

/// Reads the request on `stream` and answers what `answer` gives for it.
fn answer_with(
    mut stream: TcpStream,
    answer: &dyn Fn(&Received) -> (u16, String),
) -> io::Result<Received> {
    let request = read(&stream)?;
    let (status, body) = answer(&request);
    let location = if (300..400).contains(&status) {
        format!("Location: {body}\r\n")
    } else {
        String::new()
    };
    write!(
        stream,
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{location}Content-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    )?;
    Ok(request)
}

/// The request on `stream`: its line, headers and body.
pub(super) fn read(stream: &TcpStream) -> io::Result<Received> {
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
        bytes: body,
    })
}

/// Sends the server at `address` the request line and headers in `head`, then `part` of the
/// body they announce and nothing more, as a client that stalls, and returns all the server
/// answers until it closes the connection: for a server's limit on a body that never arrives
/// whole.
pub async fn sent_in_part(address: &str, head: &str, part: &[u8]) -> io::Result<String> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let mut stream = tokio::net::TcpStream::connect(address).await?;
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(part).await?;
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).await?;
    Ok(String::from_utf8_lossy(&answer).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_taken_before_the_stop_is_kept_however_late_it_is_answered() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let mut client = TcpStream::connect(address).unwrap();
        client.write_all(b"GET /late HTTP/1.1\r\n\r\n").unwrap();
        // The connection that wakes it to stop, taken next.
        let _wake = TcpStream::connect(address).unwrap();
        let (log, stop) = (Mutex::new(Vec::new()), AtomicBool::new(false));
        // Its requests are asked for while it answers, as a test asks once its client has the
        // answer.
        let answer = |_: &Received| {
            stop.store(true, Ordering::SeqCst);
            (200, "{}".to_owned())
        };
        serve(&listener, &answer, &log, &stop);
        let kept = log.into_inner().unwrap();
        let paths: Vec<&str> = kept.iter().map(|request| request.path.as_str()).collect();
        assert_eq!(paths, ["/late"], "kept, though answered after the stop");
    }
}
