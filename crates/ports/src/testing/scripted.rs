//! A fake HTTP server that answers each request with the next of its scripted answers, keeps
//! every request it was asked, and holds a stalled one open: the core of the model providers'
//! fake servers, each of which writes its own wire format.

use std::collections::VecDeque;
use std::io::{self, Read as _, Write as _};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use super::http::{Received, read};

/// One answer, as it goes on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Written {
    /// A whole answer: its status, its content type and its body.
    Whole {
        status: u16,
        content_type: &'static str,
        body: String,
    },
    /// No answer at all: the request is held open until the client hangs up, like a model
    /// that never starts.
    Stall,
}

/// The requests received so far, and a signal for each new one.
type Log = Arc<(Mutex<Vec<Received>>, Condvar)>;

/// The running server, on 127.0.0.1 only. Dropping it stops the server and waits for its
/// thread.
#[derive(Debug)]
pub struct ScriptedHttp {
    address: SocketAddr,
    received: Log,
    stop: Arc<AtomicBool>,
    /// Every connection taken, closed when the server stops, so a client still waiting on
    /// one, as it waits on a stall, is let go.
    open: Arc<Mutex<Vec<TcpStream>>>,
    thread: Option<JoinHandle<()>>,
}

impl ScriptedHttp {
    /// Starts a server that answers with `answers`, in order. After the last one, every
    /// request gets a 500 saying the script ran out.
    pub fn start(answers: Vec<Written>) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let answers = Mutex::new(VecDeque::from(answers));
        let received: Log = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let open: Arc<Mutex<Vec<TcpStream>>> = Arc::default();
        let thread = {
            let (received, stop, taken) =
                (Arc::clone(&received), Arc::clone(&stop), Arc::clone(&open));
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { continue };
                    let mut taken = taken.lock().unwrap_or_else(PoisonError::into_inner);
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Ok(held) = stream.try_clone() {
                        taken.push(held);
                    }
                    drop(taken);
                    // A broken exchange only fails the test that caused it.
                    let _ = serve(&stream, &answers, &received);
                }
            })
        };
        Ok(Self {
            address,
            received,
            stop,
            open,
            thread: Some(thread),
        })
    }

    /// `http://` and the address, such as `http://127.0.0.1:53211`.
    pub fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    /// The requests received so far, in order.
    pub fn received(&self) -> Vec<Received> {
        self.received
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Waits until `count` requests have arrived, or `limit` has passed; whether they did.
    pub fn wait_for_requests(&self, count: usize, limit: Duration) -> bool {
        let (requests, arrived) = &*self.received;
        let requests = requests.lock().unwrap_or_else(PoisonError::into_inner);
        arrived
            .wait_timeout_while(requests, limit, |requests| requests.len() < count)
            .is_ok_and(|(requests, _)| requests.len() >= count)
    }
}

impl Drop for ScriptedHttp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // The flag is set before the lock is taken, so a connection is either closed here or
        // never served.
        for stream in self
            .open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
        {
            let _ = stream.shutdown(Shutdown::Both);
        }
        // Wakes the accept loop, so it sees the flag.
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Reads the request on `stream`, keeps it, and answers it with the next of `answers`.
fn serve(stream: &TcpStream, answers: &Mutex<VecDeque<Written>>, received: &Log) -> io::Result<()> {
    let request = read(stream)?;
    if request.method.is_empty() {
        // A connection that sent nothing.
        return Ok(());
    }
    let answer = answers
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .pop_front()
        .unwrap_or_else(|| Written::Whole {
            status: 500,
            content_type: "application/json",
            body: "the fake server ran out of replies".into(),
        });
    let (requests, arrived) = &**received;
    requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(request);
    arrived.notify_all();
    let mut stream = stream;
    match answer {
        Written::Whole {
            status,
            content_type,
            body,
        } => {
            write!(
                stream,
                "HTTP/1.1 {status} FAKE\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\n\
                 connection: close\r\n\r\n{body}",
                body.len()
            )?;
            stream.flush()
        }
        Written::Stall => {
            // Waits for the client to hang up: a read of nothing means it did.
            let _ = stream.read(&mut [0_u8; 1]);
            Ok(())
        }
    }
}
