//! One command's connection to the proxy: its request read, its host decided, and its bytes
//! carried to the host, or a refusal said in its place.

use std::io;
use std::net::{IpAddr, SocketAddr};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio::time::timeout;

use super::head::{self, Request, Unread};
use super::network::Network;
use super::pump::pump;
use super::rules::{Host, Refusal, Step, kept};
use super::{Held, Limits, Shared};

/// How a connection ends short of its host, as its HTTP answer says.
#[derive(Debug)]
enum Answer {
    Refused(Refusal),
    BadRequest(&'static str),
    Unreachable(String),
}

/// Serves one connection: reads its request, decides, and carries it, or answers why not.
pub(crate) async fn serve<S, N>(mut client: S, shared: &Shared<N>) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
    N: Network,
{
    let limits = &shared.limits;
    let (request, early) = match timeout(limits.head, read_head(&mut client)).await {
        Ok(read) => read?,
        Err(_) => return Ok(()),
    };
    let request = match request {
        Ok(request) => request,
        Err(answer) => return answer_with(&mut client, &answer, limits).await,
    };
    let (host, port) = match &request {
        Request::Tunnel { host, port } | Request::Forward { host, port, .. } => (host, *port),
    };
    let mut upstream = match reach(host, port, shared).await {
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
async fn read_head<S: AsyncRead + Unpin>(
    client: &mut S,
) -> io::Result<(Result<Request, Answer>, Vec<u8>)> {
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
async fn reach<N: Network>(
    host: &Host,
    port: u16,
    shared: &Shared<N>,
) -> Result<TcpStream, Answer> {
    match shared.rules.step(host) {
        Step::Go => {}
        Step::Refuse(refusal) => return Err(Answer::Refused(refusal)),
        Step::Ask => hold(host, shared).await?,
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

/// Holds the connection for a yes from whoever answers the proxy's holds, up to the limit: no
/// one to ask, a no, an answer never given or the limit passed is a refusal.
async fn hold<N>(host: &Host, shared: &Shared<N>) -> Result<(), Answer> {
    let not_allowed = || Answer::Refused(Refusal::NotAllowed);
    let holds = shared.holds.as_ref().ok_or_else(not_allowed)?;
    let (answer, answered) = oneshot::channel();
    let held = Held::new(host.clone(), answer);
    holds.try_send(held).map_err(|_| not_allowed())?;
    match timeout(shared.limits.hold, answered).await {
        Ok(Ok(true)) => Ok(()),
        Ok(Ok(false) | Err(_)) | Err(_) => Err(not_allowed()),
    }
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
        Answer::BadRequest(why) => ("400 Bad Request", (*why).to_owned()),
        Answer::Unreachable(why) => ("502 Bad Gateway", why.clone()),
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
