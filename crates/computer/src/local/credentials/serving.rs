//! Answering one command's credential helper: each connection presents the command's password,
//! then git's request (`protocol=`, `host=`, `path=` lines, as git's credential protocol writes
//! them, ending with an empty line), and gets back `username=` and `password=` lines, or
//! nothing. Only hosts offered to the command are asked about; a connection that does not
//! present the password, or takes too long, gets nothing.

use std::time::Duration;

use glave_ports::auth::safe_to_send;
use glave_ports::{CredentialAsk, CredentialHost, Credentials};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{TcpListener, TcpStream};

use crate::token::same;

/// How long one request may take, from connecting to the answer.
const ANSWER: Duration = Duration::from_secs(30);
/// The most a request may hold, in bytes: git's description of a URL is a few lines.
const LONGEST: u64 = 16 * 1024;

/// Answers connections until the command ends and this is dropped. One at a time: git asks
/// rarely, and a connection that stalls gives up after [`ANSWER`].
pub(super) async fn serve(
    listener: &TcpListener,
    password: &str,
    hosts: &[CredentialHost],
    credentials: &dyn Credentials,
) {
    while let Ok((stream, peer)) = listener.accept().await {
        if !peer.ip().is_loopback() {
            continue;
        }
        let answered = answer(stream, password, hosts, credentials);
        let _ = tokio::time::timeout(ANSWER, answered).await;
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
    let given = lines.next_line().await?.unwrap_or_default();
    if !same(&given, password) {
        return Ok(());
    }
    let mut fields = Fields::default();
    while let Some(line) = lines.next_line().await? {
        if line.is_empty() {
            break;
        }
        fields.read(&line);
    }
    let Some(ask) = fields.ask() else {
        return Ok(());
    };
    if !hosts
        .iter()
        .any(|host| Place::of(&host.url).is_some_and(|place| place.is(&ask)))
    {
        return Ok(());
    }
    if let Some(credential) = credentials.request(&ask).await
        && credential.check().is_ok()
    {
        let answer = format!(
            "username={}\npassword={}\n",
            credential.username, credential.password
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

/// Where an offered host is: its scheme and its host with any port.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Place<'a> {
    scheme: &'a str,
    host: &'a str,
}

impl<'a> Place<'a> {
    /// `url`'s place, when it is `https://`, or `http://` to this computer.
    pub(super) fn of(url: &'a str) -> Option<Self> {
        let (scheme, rest) = url.split_once("://")?;
        let host = rest.split('/').next().filter(|host| !host.is_empty())?;
        safe_to_send(url, &[]).then_some(Self { scheme, host })
    }

    /// Whether `ask` is for this place.
    fn is(&self, ask: &CredentialAsk) -> bool {
        ask.protocol == self.scheme && ask.host.eq_ignore_ascii_case(self.host)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_or_this_computer_is_offered() {
        let place = Place::of("https://github.com").unwrap();
        let ask = |protocol: &str, host: &str| CredentialAsk {
            protocol: protocol.into(),
            host: host.into(),
            path: None,
        };
        assert!(place.is(&ask("https", "GitHub.com")));
        assert!(!place.is(&ask("http", "github.com")));
        assert!(!place.is(&ask("https", "github.com.evil.example")));
        assert!(Place::of("http://127.0.0.1:8080/git").is_some());
        assert!(Place::of("http://github.com").is_none());
        assert!(Place::of("http://localhost.evil.example").is_none());
        assert!(Place::of("https://").is_none());
        assert!(Place::of("github.com").is_none());
    }
}
