//! A connection held for the person: the question it puts to whoever answers the proxy's
//! holds, and its wait for the answer, which ends at the limit, when the command that asked
//! goes away, or when the question is dropped unanswered.

use std::future::pending;
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::{OwnedSemaphorePermit, oneshot};
use tokio::time::timeout;

use super::Shared;
use super::connection::Answer;
use super::head;
use super::rules::{Host, Refusal};

/// A connection held for the person: the host and port it names, and the answer it waits for.
#[derive(Debug)]
pub struct Held {
    host: Host,
    port: u16,
    answer: oneshot::Sender<bool>,
}

impl Held {
    /// The host the connection names.
    pub fn host(&self) -> &Host {
        &self.host
    }

    /// The port it names.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Lets the connection through (`yes`) or refuses it. A connection that has stopped
    /// waiting is not affected.
    pub fn answer(self, yes: bool) {
        // The connection may have stopped waiting; then no one needs the answer.
        let _ = self.answer.send(yes);
    }

    /// Ends once the connection has stopped waiting (its command went away, or its limit
    /// passed), so whoever asks the person can take the question back.
    pub async fn withdrawn(&mut self) {
        self.answer.closed().await;
    }
}

/// Holds the connection for a yes, up to the limit. While it waits it gives back its place
/// among the connections and takes one among the holds, so held connections never crowd out
/// allowed ones, and it watches `client`, keeping what it sends in `early` (up to a head's
/// size), so a command that goes away ends its question. No one to ask, too many held, a no, a
/// question dropped unanswered or the limit passed is a refusal; a yes takes a place among the
/// connections again, or is turned away busy.
pub(crate) async fn hold<S, N>(
    host: &Host,
    port: u16,
    shared: &Shared<N>,
    client: &mut S,
    early: &mut Vec<u8>,
    place: &mut Option<OwnedSemaphorePermit>,
) -> Result<(), Answer>
where
    S: AsyncRead + Unpin,
{
    let not_allowed = || Answer::Refused(Refusal::NotAllowed);
    let holds = shared.holds.as_ref().ok_or_else(not_allowed)?;
    let Ok(_holding) = Arc::clone(&shared.holding).try_acquire_owned() else {
        return Err(Answer::Busy);
    };
    let (answer, mut answered) = oneshot::channel();
    let held = Held {
        host: host.clone(),
        port,
        answer,
    };
    holds.try_send(held).map_err(|_| not_allowed())?;
    drop(place.take());
    let waited = timeout(shared.limits.hold, async {
        tokio::select! {
            answer = &mut answered => Some(answer),
            () = gone(client, early) => None,
        }
    })
    .await;
    match waited {
        Ok(Some(Ok(true))) => {}
        Ok(None) => return Err(Answer::Gone),
        Ok(Some(Ok(false) | Err(_))) | Err(_) => return Err(not_allowed()),
    }
    let again = Arc::clone(&shared.connections).try_acquire_owned();
    *place = Some(again.map_err(|_| Answer::Busy)?);
    Ok(())
}

/// Ends when `client` has closed or failed, keeping what it sends meanwhile in `early`; once
/// that holds a head's worth, it stops reading and never ends.
async fn gone<S: AsyncRead + Unpin>(client: &mut S, early: &mut Vec<u8>) {
    let mut chunk = [0_u8; 4096];
    while early.len() < head::MOST {
        match client.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => early.extend_from_slice(chunk.get(..n).unwrap_or_default()),
        }
    }
    pending::<()>().await;
}
