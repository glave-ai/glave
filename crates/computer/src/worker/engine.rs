//! The Docker Engine API over its Unix socket: one HTTP/1.1 connection per request, JSON in
//! and out, a tar archive in, and a connection upgraded to a raw stream for attaching to a
//! container. Written on hyper, already in the tree, rather than a Docker client crate.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Duration;

use bytes::Bytes;
use glave_ports::computer::ComputerError;
use http::{Method, Request, StatusCode, header};
use http_body_util::{BodyExt as _, Full};
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use serde_json::Value;
use tokio::net::UnixStream;

/// The API version spoken: Docker Engine 24 and later.
const VERSION: &str = "v1.43";
/// The most of a response read into memory.
const MAX_BODY: usize = 16 * 1024 * 1024;
/// How long Docker has to answer a call (its version, making, starting, reading or removing
/// a container, attaching to one): seconds, even on a busy machine.
pub(crate) const ANSWERS_WITHIN: Duration = Duration::from_mins(1);
/// How long Docker has to pull an image: minutes for a large one on a slow link.
pub(crate) const PULLS_WITHIN: Duration = Duration::from_mins(10);

/// The Engine at a Unix socket.
#[derive(Debug, Clone)]
pub(crate) struct Engine {
    socket: PathBuf,
}

/// An answer: its status and body.
pub(crate) struct Answer {
    pub(crate) status: StatusCode,
    pub(crate) body: Bytes,
}

impl Answer {
    /// The body as JSON, or `null` when it is not JSON.
    pub(crate) fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    /// The Engine's own message, for an error.
    pub(crate) fn message(&self) -> String {
        self.json()
            .get("message")
            .and_then(Value::as_str)
            .map_or_else(
                || String::from_utf8_lossy(&self.body).trim().to_owned(),
                str::to_owned,
            )
    }
}

fn unreachable(socket: &std::path::Path, error: &dyn std::fmt::Display) -> ComputerError {
    ComputerError::Unreachable(format!(
        "could not reach Docker at {}: {error}. Is Docker running?",
        socket.display()
    ))
}

impl Engine {
    pub(crate) fn new(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
        }
    }

    /// Sends one request and reads its answer whole, within `limit`; `body` is JSON when
    /// `kind` is `None`, else bytes of that type.
    pub(crate) async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<(Bytes, &str)>,
        limit: Duration,
    ) -> Result<Answer, ComputerError> {
        tokio::time::timeout(limit, self.asked(method, path, body))
            .await
            .unwrap_or_else(|_| Err(self.late(path, limit)))
    }

    /// Docker did not answer `path` within `limit`.
    fn late(&self, path: &str, limit: Duration) -> ComputerError {
        ComputerError::Unreachable(format!(
            "Docker at {} did not answer {path} within {}",
            self.socket.display(),
            glave_ports::spoken(limit)
        ))
    }

    async fn asked(
        &self,
        method: Method,
        path: &str,
        body: Option<(Bytes, &str)>,
    ) -> Result<Answer, ComputerError> {
        let request = Self::request(method, path, body, false)?;
        let stream = UnixStream::connect(&self.socket)
            .await
            .map_err(|error| unreachable(&self.socket, &error))?;
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|error| unreachable(&self.socket, &error))?;
        // The connection is driven beside the request and ends with it.
        let driving = async {
            connection
                .await
                .map_err(|error| unreachable(&self.socket, &error))
        };
        let asking = async {
            let response = sender
                .send_request(request)
                .await
                .map_err(|error| unreachable(&self.socket, &error))?;
            let status = response.status();
            let body = http_body_util::Limited::new(response.into_body(), MAX_BODY)
                .collect()
                .await
                .map_err(|error| unreachable(&self.socket, &error))?
                .to_bytes();
            drop(sender);
            Ok::<_, ComputerError>(Answer { status, body })
        };
        let (answer, _) = tokio::join!(asking, driving);
        answer
    }

    /// Sends JSON and reads JSON, within [`ANSWERS_WITHIN`].
    pub(crate) async fn json(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Answer, ComputerError> {
        let body = body.map(|value| (Bytes::from(value.to_string()), "application/json"));
        self.send(method, path, body, ANSWERS_WITHIN).await
    }

    /// Asks for a connection upgraded to a raw stream (attaching to a container), within
    /// [`ANSWERS_WITHIN`]; the stream then lasts as long as the worker's link.
    pub(crate) async fn upgrade(&self, path: &str) -> Result<TokioIo<Upgraded>, ComputerError> {
        tokio::time::timeout(ANSWERS_WITHIN, self.upgraded(path))
            .await
            .unwrap_or_else(|_| Err(self.late(path, ANSWERS_WITHIN)))
    }

    async fn upgraded(&self, path: &str) -> Result<TokioIo<Upgraded>, ComputerError> {
        let request = Self::request(Method::POST, path, None, true)?;
        let stream = UnixStream::connect(&self.socket)
            .await
            .map_err(|error| unreachable(&self.socket, &error))?;
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|error| unreachable(&self.socket, &error))?;
        let driving = async {
            connection
                .with_upgrades()
                .await
                .map_err(|error| unreachable(&self.socket, &error))
        };
        let asking = async {
            let response = sender
                .send_request(request)
                .await
                .map_err(|error| unreachable(&self.socket, &error))?;
            if response.status() != StatusCode::SWITCHING_PROTOCOLS {
                return Err(ComputerError::Failed(format!(
                    "Docker did not attach ({}): {path}",
                    response.status()
                )));
            }
            hyper::upgrade::on(response)
                .await
                .map_err(|error| unreachable(&self.socket, &error))
        };
        let (upgraded, _) = tokio::join!(asking, driving);
        upgraded.map(TokioIo::new)
    }

    fn request(
        method: Method,
        path: &str,
        body: Option<(Bytes, &str)>,
        upgrade: bool,
    ) -> Result<Request<Full<Bytes>>, ComputerError> {
        let mut builder = Request::builder()
            .method(method)
            .uri(format!("/{VERSION}{path}"))
            .header(header::HOST, "docker");
        if upgrade {
            builder = builder
                .header(header::CONNECTION, "Upgrade")
                .header(header::UPGRADE, "tcp");
        }
        let body = match body {
            Some((bytes, kind)) => {
                builder = builder.header(header::CONTENT_TYPE, kind);
                bytes
            }
            None => Bytes::new(),
        };
        builder
            .body(Full::new(body))
            .map_err(|error| ComputerError::Failed(format!("a request to Docker: {error}")))
    }
}

/// `text` escaped for a URL's query.
pub(crate) fn query(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// The standard output of a stream Docker multiplexes (8-byte frame headers: the stream, three
/// zero bytes, and the length), and its standard error, from `bytes` read whole.
pub(crate) fn demultiplex(mut bytes: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    while let Some((head, rest)) = bytes.split_first_chunk::<8>() {
        let [kind, _, _, _, a, b, c, d] = *head;
        let length = usize::try_from(u32::from_be_bytes([a, b, c, d]))
            .unwrap_or(usize::MAX)
            .min(rest.len());
        let (frame, after) = rest.split_at(length);
        if kind == 2 {
            err.extend_from_slice(frame);
        } else {
            out.extend_from_slice(frame);
        }
        bytes = after;
    }
    (out, err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_split_into_output_and_errors() {
        let mut stream = vec![1, 0, 0, 0, 0, 0, 0, 3];
        stream.extend_from_slice(b"abc");
        stream.extend_from_slice(&[2, 0, 0, 0, 0, 0, 0, 2]);
        stream.extend_from_slice(b"no");
        stream.extend_from_slice(&[1, 0, 0, 0, 0, 0, 0, 9, b'd']);
        assert_eq!(demultiplex(&stream), (b"abcd".to_vec(), b"no".to_vec()));
        assert_eq!(query("alpine@sha256:ab/c"), "alpine%40sha256%3Aab%2Fc");
    }

    #[test]
    fn docker_that_takes_a_call_and_never_answers_fails_it_at_its_limit() {
        let dir = glave_ports::testing::TempDir::new("engine-silent").unwrap();
        let socket = dir.join("docker.sock");
        let engine = Engine::new(&socket);
        let (asked, attached) = glave_ports::block_on!(async {
            tokio::time::pause();
            let listener = tokio::net::UnixListener::bind(&socket).unwrap();
            // It takes each connection and says nothing, as a stuck engine would.
            let stuck = tokio::spawn(async move {
                let mut held = Vec::new();
                while let Ok((stream, _)) = listener.accept().await {
                    held.push(stream);
                }
            });
            let asked = engine.json(Method::GET, "/version", None).await;
            let attached = engine.upgrade("/containers/w/attach").await;
            stuck.abort();
            (asked, attached.map(drop))
        });
        let said = |path: &str| {
            format!(
                "unreachable: Docker at {} did not answer {path} within 1 minute",
                socket.display()
            )
        };
        assert_eq!(asked.err().map(|e| e.to_string()), Some(said("/version")));
        assert_eq!(
            attached.err().map(|e| e.to_string()),
            Some(said("/containers/w/attach"))
        );
    }
}
