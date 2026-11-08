//! Where the processes of this machine's commands in the background listen: each TCP address
//! this machine's own loopback reaches, a loopback address or every address, as it was bound
//! (`specs/computers.md`, "Background commands"). A command's processes are those in its
//! process group and everything they started, followed by parent, as the sandbox's own session
//! on Linux leaves the group and a child may start a group of its own; a process that left on
//! purpose (a daemon, whose parent is no longer the command's) is not followed, as stopping one
//! does not follow it. On Linux it is read from `/proc` (`proc`); on macOS from the system's own
//! `ps` and `lsof`, by their paths (`lsof`); elsewhere nothing is found. Each look is bounded:
//! on Linux the walk stops at its deadline, on macOS each program is stopped at its limit.

#[cfg(any(target_os = "macos", test))]
mod lsof;
#[cfg(any(target_os = "linux", test))]
mod proc;

use std::collections::{BTreeMap, BTreeSet};
#[cfg(any(target_os = "linux", target_os = "macos", test))]
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use glave_ports::host::is_loopback;
use glave_process::Processes;

/// How long a look may take: a walk through `/proc`, or each of `ps` and `lsof`.
const LOOKING: Duration = Duration::from_secs(5);

/// Each address found, by group, then port first, so a listing reads lowest port first.
type Found = BTreeMap<u32, BTreeSet<(u16, IpAddr)>>;

/// Where each of `groups`, a process group by its leader's id, listens, by port, then address;
/// a group listening nowhere is left out.
pub(super) async fn addresses(
    groups: &[u32],
    processes: &Processes,
) -> BTreeMap<u32, Vec<SocketAddr>> {
    if groups.is_empty() {
        return BTreeMap::new();
    }
    found(groups, processes)
        .await
        .into_iter()
        .map(|(group, at)| {
            let at = at.into_iter().map(|(port, ip)| SocketAddr::new(ip, port));
            (group, at.collect())
        })
        .collect()
}

#[cfg(target_os = "linux")]
async fn found(groups: &[u32], _: &Processes) -> Found {
    let groups = groups.to_vec();
    let until = std::time::Instant::now() + LOOKING;
    // The walk stops at `until` by itself, so waiting for it is bounded and nothing goes on
    // unowned.
    tokio::task::spawn_blocking(move || proc::found(&groups, until))
        .await
        .unwrap_or_default()
}

#[cfg(target_os = "macos")]
async fn found(groups: &[u32], processes: &Processes) -> Found {
    lsof::found(groups, processes).await
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
async fn found(_: &[u32], _: &Processes) -> Found {
    BTreeMap::new()
}

/// Whether a socket listening at `address` is reached at this machine's loopback.
fn reached_here(address: IpAddr) -> bool {
    address.is_unspecified() || is_loopback(address)
}

/// A process: its parent and its group.
#[cfg(any(target_os = "linux", target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Process {
    parent: u32,
    group: u32,
}

/// The deepest a command's processes are followed from its group, parent by parent.
#[cfg(any(target_os = "linux", target_os = "macos", test))]
const DEEPEST: usize = 64;

/// Which of `groups` each of `processes` belongs to: its own group, or the first group of a
/// process above it.
#[cfg(any(target_os = "linux", target_os = "macos", test))]
fn owners(processes: &HashMap<u32, Process>, groups: &[u32]) -> HashMap<u32, u32> {
    let groups: HashSet<u32> = groups.iter().copied().collect();
    let owner = |pid: u32| {
        let mut at = processes.get(&pid)?;
        for _ in 0..DEEPEST {
            if groups.contains(&at.group) {
                return Some(at.group);
            }
            at = processes.get(&at.parent)?;
        }
        None
    };
    processes
        .keys()
        .filter_map(|pid| owner(*pid).map(|group| (*pid, group)))
        .collect()
}

#[cfg(test)]
mod tests;
