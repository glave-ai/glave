//! A faulty link between `RemoteComputers` and `serve`: two in-memory pipes with a pump on
//! each direction that passes messages (lines) along, except where a schedule says to drop
//! the connection, stall, cut a message off, splice it into the next, send it twice, swap it
//! with the next, flip a bit in it, or kill the serving side. Stalls run on tokio's clock,
//! which tests pause, so they cost no real time.

use std::sync::Arc;
use std::time::Duration;

use glave_computer::{RemoteComputers, serve};
use glave_ports::computer::Computers;
use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader};
use tokio::sync::watch;
use tokio::task::{AbortHandle, JoinHandle};

/// What happens to one message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// The connection drops before the message is passed on.
    Drop,
    /// The message is held this long, then passed on.
    Stall(Duration),
    /// Half the message is passed on, then the connection drops.
    Truncate,
    /// Half the message is passed on, without its end, and the next runs into it.
    Splice,
    /// The message is passed on twice.
    Duplicate,
    /// The message is held until the next one has gone (or a second has passed).
    Reorder,
    /// One byte of the message (chosen by the number) has its high bit flipped.
    Corrupt(usize),
    /// The serving side ends (its task is killed) before the message is passed on.
    Kill,
}

/// Which way a message goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Way {
    /// From the client to the computer: requests.
    Out,
    /// From the computer to the client: answers and output.
    Back,
}

/// One fault: what happens to the `at`-th message (from 0) going `way`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Planned {
    pub way: Way,
    pub at: usize,
    pub fault: Fault,
}

/// The pumps and the serving side, stopped when this is dropped.
pub struct Link {
    tasks: Vec<JoinHandle<()>>,
}

impl Drop for Link {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// Serves `computers` across a link that applies `faults`, and the client at its other end.
pub fn faulty<C: Computers + 'static>(
    computers: Arc<C>,
    faults: &[Planned],
) -> (RemoteComputers, Link) {
    let (client, client_side) = tokio::io::duplex(1 << 20);
    let (server_side, server) = tokio::io::duplex(1 << 20);
    let (server_read, server_write) = tokio::io::split(server);
    let serving = tokio::spawn(async move {
        let _ = serve(computers, BufReader::new(server_read), server_write, None).await;
    });
    let (dropped, watching) = watch::channel(false);
    let dropped = Arc::new(dropped);
    let (from_client, to_client) = tokio::io::split(client_side);
    let (from_server, to_server) = tokio::io::split(server_side);
    let plan = |way| {
        faults
            .iter()
            .filter(|planned| planned.way == way)
            .copied()
            .collect::<Vec<_>>()
    };
    let out = Pump {
        plan: plan(Way::Out),
        dropped: Arc::clone(&dropped),
        watching: watching.clone(),
        serving: serving.abort_handle(),
    };
    let back = Pump {
        plan: plan(Way::Back),
        dropped,
        watching,
        serving: serving.abort_handle(),
    };
    let out = tokio::spawn(out.run(BufReader::new(from_client), to_server));
    let back = tokio::spawn(back.run(BufReader::new(from_server), to_client));
    let (read, write) = tokio::io::split(client);
    let remote = RemoteComputers::connect(read, write);
    let link = Link {
        tasks: vec![serving, out, back],
    };
    (remote, link)
}

/// Moves messages one way, applying its faults.
struct Pump {
    plan: Vec<Planned>,
    dropped: Arc<watch::Sender<bool>>,
    watching: watch::Receiver<bool>,
    serving: AbortHandle,
}

impl Pump {
    fn fault(&self, at: usize) -> Option<Fault> {
        self.plan
            .iter()
            .find(|planned| planned.at == at)
            .map(|planned| planned.fault)
    }

    /// Passes lines from `from` to `to` until either end closes or a fault drops the link.
    async fn run<R, W>(self, mut from: BufReader<R>, mut to: W)
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        let mut held: Option<Vec<u8>> = None;
        let mut at = 0;
        loop {
            let mut line = Vec::new();
            let mut watching = self.watching.clone();
            let dropped = async move {
                let _ = watching.wait_for(|dropped| *dropped).await;
            };
            let next = tokio::select! {
                read = from.read_until(b'\n', &mut line) => Some(read),
                () = dropped => None,
                () = tokio::time::sleep(Duration::from_secs(1)), if held.is_some() => {
                    // Nothing came to swap with: the held message goes on alone.
                    if let Some(message) = held.take() && to.write_all(&message).await.is_err() {
                        break;
                    }
                    continue;
                }
            };
            match next {
                Some(Ok(got)) if got > 0 => {}
                _ => break,
            }
            let fault = self.fault(at);
            at += 1;
            let keep = match fault {
                None => send(&mut to, &line).await,
                Some(Fault::Drop) => false,
                Some(Fault::Kill) => {
                    self.serving.abort();
                    send(&mut to, &line).await
                }
                Some(Fault::Stall(time)) => {
                    tokio::time::sleep(time).await;
                    send(&mut to, &line).await
                }
                Some(Fault::Truncate) => {
                    let _ = to
                        .write_all(line.get(..line.len() / 2).unwrap_or_default())
                        .await;
                    false
                }
                Some(Fault::Splice) => {
                    send(&mut to, line.get(..line.len() / 2).unwrap_or_default()).await
                }
                Some(Fault::Duplicate) => send(&mut to, &line).await && send(&mut to, &line).await,
                Some(Fault::Reorder) => {
                    held = Some(line);
                    continue;
                }
                Some(Fault::Corrupt(which)) => {
                    // Never the newline: the message stays one message, with a bad byte.
                    let body = line.len().saturating_sub(1).max(1);
                    if let Some(byte) = line.get_mut(which % body) {
                        *byte ^= 0x80;
                    }
                    send(&mut to, &line).await
                }
            };
            if !keep {
                break;
            }
            if let Some(message) = held.take()
                && !send(&mut to, &message).await
            {
                break;
            }
        }
        // Either end gone, or a fault: the whole link drops, both ways.
        let _ = self.dropped.send(true);
        let _ = to.shutdown().await;
    }
}

async fn send<W: AsyncWrite + Unpin>(to: &mut W, bytes: &[u8]) -> bool {
    to.write_all(bytes).await.is_ok() && to.flush().await.is_ok()
}
