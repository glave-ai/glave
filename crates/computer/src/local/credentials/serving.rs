//! Answering one command's credential helper: each connection presents the command's password,
//! then git's request (`protocol=`, `host=`, `path=` lines, as git's credential protocol writes
//! them, ending with an empty line), and gets back `username=` and `password=` lines, or
//! nothing. Only hosts offered to the command are asked about; a connection that does not
//! present the password at once, or takes too long, gets nothing.

use std::time::Duration;

use glave_ports::{CredentialAsk, CredentialHost, Credentials};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{TcpListener, TcpStream};

use crate::token::same;

/// How long a connection has to present the password.
const PASSWORD: Duration = Duration::from_secs(2);
/// How long one request may take once the password is presented.
const ANSWER: Duration = Duration::from_secs(30);
/// How long to wait after a connection could not be accepted, before trying again.
const AGAIN: Duration = Duration::from_millis(50);
/// The most a request may hold, in bytes: git's description of a URL is a few lines.
const LONGEST: u64 = 16 * 1024;

/// Answers connections until the command ends and this is dropped, one at a time: git asks
/// rarely, and a connection that stalls gives up after [`PASSWORD`] unless it knows it.
pub(super) async fn serve(
    listener: &TcpListener,
    password: &str,
    hosts: &[CredentialHost],
    credentials: &dyn Credentials,
) {
    loop {
        let Ok((stream, peer)) = listener.accept().await else {
            // Such as too many open files: the helper waits, and this tries again.
            tokio::time::sleep(AGAIN).await;
            continue;
        };
        if !peer.ip().is_loopback() {
            continue;
        }
        let _ = answer(stream, password, hosts, credentials).await;
    }
}

/// Reads one request and answers it, if it carries the password and names an offered host.
async fn answer(
    stream: TcpStream,
    password: &str,
    hosts: &[CredentialHost],
    credentials: &dyn Credentials,
) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read.take(LONGEST)).lines();
    let given = tokio::time::timeout(PASSWORD, lines.next_line()).await;
    if !matches!(given, Ok(Ok(Some(given))) if same(&given, password)) {
        return Ok(());
    }
    let asked = async {
        let mut fields = Fields::default();
        while let Some(line) = lines.next_line().await? {
            if line.is_empty() {
                break;
            }
            fields.read(&line);
        }
        let Some(ask) = fields.ask() else {
            return Ok(None);
        };
        if !hosts.iter().any(|host| host.is_for(&ask)) {
            return Ok(None);
        }
        Ok::<_, std::io::Error>(credentials.request(&ask).await)
    };
    let Ok(answered) = tokio::time::timeout(ANSWER, asked).await else {
        return Ok(());
    };
    if let Some(credential) = answered?
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
