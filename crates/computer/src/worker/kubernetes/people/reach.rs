//! How a server reaches a person's computer (`specs/computers.md`, decision 18): one served,
//! whichever server woke it, through a connection of its own, never replacing it however often
//! that fails; one not awake, or served by a glave of another protocol, woken under its lease;
//! and, while another server holds the lease of one not served, a wait, as long as that server's
//! wake goes on.

use std::time::Duration;

use glave_event::PrincipalId;
use glave_ports::OrgId;
use glave_ports::computer::{ComputerError, WorkspaceInfo};
use tokio::time::Instant;

use super::super::served::Seen;
use super::lease::{self, Held, Leasing};

/// How often a server waiting for another to wake a person's computer, or to let go of its
/// lease, or for a served computer to answer, looks again.
const LOOK_AGAIN: Duration = Duration::from_secs(2);

/// A person's computer as one server reaches it.
pub(super) trait Machine {
    /// Its Pod, as the cluster shows it.
    async fn seen(&self) -> Result<Seen, ComputerError>;
    /// Reaches the served Pod `pod` and opens the person's home there.
    async fn attach(&self, pod: &str) -> Result<WorkspaceInfo, ComputerError>;
    /// Wakes it, under its lease, replacing what is left of it, and opens the person's home.
    async fn wake(&self) -> Result<WorkspaceInfo, ComputerError>;
    /// Keeps `held`, its lease, renewed while it wakes or is awake.
    fn keep(&self, held: Held);
}

/// `machine`, the computer of `person` in `org`, reached and the person's home opened on it,
/// within `wait`: at most that long while another server's wake goes on, and
/// [`lease::LEASE_WAIT`] once nothing moves (a holder that hasn't found the computer slept, a
/// served computer that doesn't answer).
pub(super) async fn reached(
    machine: &impl Machine,
    leasing: &Leasing,
    (org, person): (OrgId, PrincipalId),
    wait: Duration,
) -> Result<WorkspaceInfo, ComputerError> {
    let started = Instant::now();
    let mut moved = started;
    loop {
        let unanswered = match machine.seen().await {
            Ok(seen) => {
                if let Some(pod) = seen.speaks_ours() {
                    // Served: reached, or tried again, never replaced.
                    match machine.attach(pod).await {
                        Ok(home) => {
                            if let Ok(Some(held)) = leasing.take(org, person).await {
                                machine.keep(held);
                            }
                            return Ok(home);
                        }
                        Err(error) => Some(error),
                    }
                } else if let Some(held) = leasing.take(org, person).await? {
                    // Not awake, served by another protocol, or left waking by a server whose
                    // lease went with it.
                    machine.keep(held);
                    return machine.wake().await;
                } else {
                    if seen == Seen::Waking {
                        // Another server is waking it.
                        moved = Instant::now();
                    }
                    None
                }
            }
            Err(error) => Some(error),
        };
        let now = Instant::now();
        if now >= started + wait || now >= moved + lease::LEASE_WAIT {
            return Err(match unanswered {
                Some(error) => ComputerError::Failed(format!(
                    "your computer is awake but this server could not reach it: {}",
                    error.message()
                )),
                None => ComputerError::Failed(lease::ELSEWHERE.into()),
            });
        }
        tokio::time::sleep(LOOK_AGAIN).await;
    }
}

#[cfg(test)]
mod tests;
