//! What runs beside a terminal while it is open: reading what its shell prints into its ring,
//! and ending it, with everything its shell started, once it is hung up, nothing used it for
//! [`IDLE`], or its shell exits.

use std::io::ErrorKind;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

#[cfg(doc)]
use glave_ports::computer::IDLE;
use glave_process::Child;
use tokio::net::unix::pipe;
use tokio::time::Instant;

use super::Shared;

/// How much is read from a terminal at once.
const READ_AT_ONCE: usize = 16 * 1024;

/// How long what a shell printed as it exited has to be read before its terminal ends.
const DRAIN: Duration = Duration::from_millis(500);

/// How the pump stopped reading the terminal.
enum Stopped {
    /// It was closed, or nothing used it for [`IDLE`].
    HungUp,
    /// Its shell exited.
    Exited,
    /// Its shell's side of the terminal closed.
    Closed,
}

/// Reads what the terminal's shell prints into its ring until it is hung up, its shell exits
/// or its side closes; then ends its session and says the terminal ended.
pub(super) async fn pump(shared: Arc<Shared>, output: pipe::Receiver, mut child: Child) {
    let mut buffer = vec![0_u8; READ_AT_ONCE];
    let stopped = loop {
        let idle_at = shared.idle_at();
        tokio::select! {
            ready = output.readable() => match ready.and_then(|()| output.try_read(&mut buffer)) {
                Ok(read) if read > 0 => shared.print(buffer.get(..read).unwrap_or_default()),
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                // Its other side closed: the end (Linux says so as an error).
                Ok(_) | Err(_) => break Stopped::Closed,
            },
            _ = child.wait() => break Stopped::Exited,
            () = shared.hang_up.notified() => break Stopped::HungUp,
            () = tokio::time::sleep_until(idle_at) => {
                if shared.idle_at() <= Instant::now() {
                    break Stopped::HungUp;
                }
            }
        }
    };
    match stopped {
        Stopped::HungUp | Stopped::Closed => {
            shared.session.hang_up().await;
            child.stop().await;
        }
        Stopped::Exited => {
            let _ = tokio::time::timeout(DRAIN, drain(&shared, &output, &mut buffer)).await;
            // What it started is left, in jobs of their own: they end with it.
            shared.session.hang_up().await;
            child.stop_left().await;
        }
    }
    shared.ended.store(true, Ordering::SeqCst);
    shared.end();
}

/// Reads what is left to read, until the terminal's other side closes.
async fn drain(shared: &Shared, output: &pipe::Receiver, buffer: &mut [u8]) {
    loop {
        match output
            .readable()
            .await
            .and_then(|()| output.try_read(buffer))
        {
            Ok(read) if read > 0 => shared.print(buffer.get(..read).unwrap_or_default()),
            Err(error) if error.kind() == ErrorKind::WouldBlock => {}
            Ok(_) | Err(_) => return,
        }
    }
}

/// Writes all of `input` to the terminal.
pub(super) async fn write_all(terminal: &pipe::Sender, input: &[u8]) -> std::io::Result<()> {
    let mut rest = input;
    while !rest.is_empty() {
        terminal.writable().await?;
        match terminal.try_write(rest) {
            Ok(written) => rest = rest.get(written..).unwrap_or_default(),
            Err(error) if error.kind() == ErrorKind::WouldBlock => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
