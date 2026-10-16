//! Answering one command's credential helper: each connection presents the command's password,
//! then git's request (`protocol=`, `host=`, `path=` lines, as git's credential protocol writes
//! them, ending with an empty line), and gets back `username=` and `password=` lines, or
//! nothing. Only hosts offered to the command are asked about; a connection that does not
//! present the password at once, or takes too long, gets nothing, and holds no one else up.

use std::collections::VecDeque;
use std::time::Duration;

use glave_ports::{CredentialAsk, CredentialHost, Credentials};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{AbortHandle, JoinSet};

use crate::token::same;

/// How long a connection has to present the password.
const PASSWORD: Duration = Duration::from_secs(2);
/// How long the rest of a request may take to arrive.
const REQUEST: Duration = Duration::from_secs(10);
/// How long the caller has to answer one request.
const ANSWER: Duration = Duration::from_secs(30);
/// How long to wait after a connection could not be accepted, before trying again.
const AGAIN: Duration = Duration::from_millis(50);
/// Connections read at once; past this, the oldest is ended.
const READING: usize = 8;
/// The most a request may hold, in bytes: git's description of a URL is a few lines.
const LONGEST: u64 = 16 * 1024;

/// Answers connections until the command ends and this is dropped, which stops every read
/// still going. Requests are read side by side and answered one at a time; with
/// [`READING`] reads going, a new connection ends the oldest, so stalled ones never shut the
/// helper out.
pub(super) async fn serve(
    listener: &TcpListener,
    password: &str,
    hosts: &[CredentialHost],
    credentials: &dyn Credentials,
) {
    let mut reading = JoinSet::new();
    let mut oldest: VecDeque<AbortHandle> = VecDeque::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) if peer.ip().is_loopback() => {
                    oldest.push_back(reading.spawn(read(stream, password.to_owned(), hosts.to_vec())));
                    if oldest.len() > READING && let Some(ended) = oldest.pop_front() {
                        ended.abort();
                    }
                }
                Ok(_) => {}
                // Such as too many open files: the helper waits, and this tries again.
                Err(_) => tokio::time::sleep(AGAIN).await,
            },
            Some(read) = reading.join_next(), if !reading.is_empty() => {
                if let Ok(Some((write, ask))) = read {
                    let _ = tokio::time::timeout(ANSWER, answer(write, &ask, credentials)).await;
                }
            }
        }
    }
}

/// Reads one request: what it asks, when it carries the password and names an offered host.
async fn read(
    stream: TcpStream,
    password: String,
    hosts: Vec<CredentialHost>,
) -> Option<(OwnedWriteHalf, CredentialAsk)> {
    let (read, write) = stream.into_split();
    let mut lines = BufReader::new(read.take(LONGEST)).lines();
    let given = tokio::time::timeout(PASSWORD, lines.next_line()).await;
    if !matches!(given, Ok(Ok(Some(given))) if same(&given, &password)) {
        return None;
    }
    let fields = async {
        let mut fields = Fields::default();
        while let Ok(Some(line)) = lines.next_line().await {
            if line.is_empty() {
                break;
            }
            fields.read(&line);
        }
        fields
    };
    let ask = tokio::time::timeout(REQUEST, fields).await.ok()?.ask()?;
    hosts
        .iter()
        .any(|host| host.is_for(&ask))
        .then_some((write, ask))
}

/// Answers `ask` from `credentials`, or with nothing.
async fn answer(
    mut write: OwnedWriteHalf,
    ask: &CredentialAsk,
    credentials: &dyn Credentials,
) -> std::io::Result<()> {
    if let Some(credential) = credentials.request(ask).await
        && credential.check().is_ok()
    {
        let answer = format!(
            "username={}\npassword={}\n",
            credential.username,
            credential.password.expose()
        );
        write.write_all(answer.as_bytes()).await?;
    }
    write.shutdown().await
}

/// The fields of git's request this answer needs.
#[derive(Debug, Default)]
struct Fields {
    protocol: Option<String>,
    host: Option<String>,
    path: Option<String>,
}

impl Fields {
    fn read(&mut self, line: &str) {
        let Some((key, value)) = line.split_once('=') else {
            return;
        };
        let value = Some(value.to_owned());
        match key {
            "protocol" => self.protocol = value,
            "host" => self.host = value,
            "path" => self.path = value,
            _ => {}
        }
    }

    fn ask(self) -> Option<CredentialAsk> {
        Some(CredentialAsk {
            protocol: self.protocol?,
            host: self.host?,
            path: self.path,
        })
    }
}
