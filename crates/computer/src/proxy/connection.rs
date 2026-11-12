//! One command's connection to the proxy: its request read, its host decided, and its bytes
//! carried to the host, or a refusal said in its place.

use std::io;
use std::net::{IpAddr, SocketAddr};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::timeout;

use super::head::{self, Credential, Request, Unread};
use super::hold::hold;
use super::network::Network;
use super::pump::pump;
use super::rules::{Host, Refusal, Step, kept};
use super::{Limits, Shared};
use crate::token::same;

/// A head read: the request and the credential it presents, or the answer it gets.
type Read = Result<(Request, Option<Credential>), Answer>;

/// How a connection ends short of its host, as its HTTP answer says.
#[derive(Debug)]
pub(crate) enum Answer {
    /// Refused by the rules, saying why.
    Refused(Refusal),
    /// Without the proxy's credential.
    Unauthorized,
    /// Not a request the proxy takes.
    BadRequest(&'static str),
    /// Too many connections, or too many held.
    Busy,
    /// The host could not be reached.
    Unreachable(String),
    /// The command went away while its connection was held: no one to answer.
    Gone,
}

/// Serves one connection, which holds `place` among the proxy's connections: reads its
/// request, checks the proxy's credential, decides, and carries it, or answers why not.
pub(crate) async fn serve<S, N>(
    mut client: S,
    shared: &Shared<N>,
    place: OwnedSemaphorePermit,
) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
    N: Network,
{
    let mut place = Some(place);
    let limits = &shared.limits;
    let (request, mut early) = match timeout(limits.head, read_head(&mut client)).await {
        Ok(read) => read?,
        Err(_) => return Ok(()),
    };
    let request = match request {
        Ok((request, credential))
            if credential
                .as_ref()
                .is_some_and(|c| same(c.presented(), &shared.secret)) =>
        {
            request
        }
        Ok(_) => return answer_with(&mut client, &Answer::Unauthorized, limits).await,
        Err(answer) => return answer_with(&mut client, &answer, limits).await,
    };
    let (host, port) = match &request {
        Request::Tunnel { host, port } | Request::Forward { host, port, .. } => (host, *port),
    };
    let reached = reach(host, port, shared, &mut client, &mut early, &mut place).await;
    let mut upstream = match reached {
        Ok(upstream) => upstream,
        Err(answer) => {
            tracing::debug!(%host, port, ?answer, "a command's connection was refused");
            return answer_with(&mut client, &answer, limits).await;
        }
    };
    match &request {
        Request::Tunnel { .. } => {
            let established = b"HTTP/1.1 200 Connection established\r\n\r\n";
            send(&mut client, established, limits).await?;
        }
        Request::Forward { head, .. } => send(&mut upstream, head, limits).await?,
    }
    send(&mut upstream, &early, limits).await?;
    pump(&mut client, &mut upstream, limits.idle).await
}

/// The request's head and whatever came after it, or the answer a head that is not a request
/// gets.
async fn read_head<S: AsyncRead + Unpin>(client: &mut S) -> io::Result<(Read, Vec<u8>)> {
    let mut read = Vec::with_capacity(1024);
    let mut chunk = [0_u8; 4096];
    loop {
        if let Some(end) = head::end(&read) {
            let early = read.split_off(end);
            let request = head::parse(&read).map_err(|unread| {
                Answer::BadRequest(match unread {
                    Unread::Malformed => "This is not a request a proxy takes.",
                    Unread::Scheme => "Only http is sent through as it is; use CONNECT for https.",
                })
            });
            return Ok((request, early));
        }
        if read.len() > head::MOST {
            return Ok((
                Err(Answer::BadRequest("The request's head is too large.")),
                Vec::new(),
            ));
        }
        let n = client.read(&mut chunk).await?;
        let Some(bytes) = chunk.get(..n).filter(|bytes| !bytes.is_empty()) else {
            return Err(io::ErrorKind::UnexpectedEof.into());
        };
        read.extend_from_slice(bytes);
    }
}

/// A connection to `host`, if the rules let it through: decided by name, held for the person
/// where they say to ask, looked up, checked, and connected to an address that passed.
async fn reach<S: AsyncRead + Unpin, N: Network>(
    host: &Host,
    port: u16,
    shared: &Shared<N>,
    client: &mut S,
    early: &mut Vec<u8>,
    place: &mut Option<OwnedSemaphorePermit>,
) -> Result<TcpStream, Answer> {
    match shared.rules.step(host, port) {
        Step::Go => {}
        Step::Refuse(refusal) => return Err(Answer::Refused(refusal)),
        Step::Ask => hold(host, port, shared, client, early, place).await?,
    }
    let limits = &shared.limits;
    let found = match host {
        Host::Address(address) => vec![*address],
        Host::Name(name) => match timeout(limits.resolve, shared.network.resolve(name)).await {
            Ok(Ok(found)) => found,
            Ok(Err(_)) => Vec::new(),
            Err(_) => {
                return Err(Answer::Unreachable(format!(
                    "Looking {host} up took longer than {}.",
                    glave_ports::spoken(limits.resolve)
                )));
            }
        },
    };
    let addresses: Vec<IpAddr> = kept(&found).map_err(Answer::Refused)?;
    let mut last = String::new();
    for address in addresses {
        let to = SocketAddr::new(address, port);
        match timeout(limits.connect, shared.network.connect(to)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(error)) => last = format!("{host} could not be reached: {error}."),
            Err(_) => {
                last = format!(
                    "{host} did not answer within {}.",
                    glave_ports::spoken(limits.connect)
                );
            }
        }
    }
    Err(Answer::Unreachable(last))
}

/// Writes `bytes` to `to`, which must take them within the limit on reading a head.
async fn send<S: AsyncWrite + Unpin>(to: &mut S, bytes: &[u8], limits: &Limits) -> io::Result<()> {
    timeout(limits.head, to.write_all(bytes))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "the other side stopped reading"))?
}

/// Says `answer` as an HTTP response and closes.
async fn answer_with<S: AsyncWrite + Unpin>(
    client: &mut S,
    answer: &Answer,
    limits: &Limits,
) -> io::Result<()> {
    let (status, body) = match answer {
        Answer::Refused(refusal) => ("403 Forbidden", refusal.to_string()),
        Answer::Unauthorized => (
            "407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"glave\"",
            "This proxy is for the commands glave runs, with the address it gave them.".to_owned(),
        ),
        Answer::BadRequest(why) => ("400 Bad Request", (*why).to_owned()),
        Answer::Busy => (
            "503 Service Unavailable",
            "Too many connections are open or waiting; try again shortly.".to_owned(),
        ),
        Answer::Unreachable(why) => ("502 Bad Gateway", why.clone()),
        Answer::Gone => return Ok(()),
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    send(client, response.as_bytes(), limits).await?;
    timeout(limits.head, client.shutdown())
        .await
        .unwrap_or(Ok(()))
}
