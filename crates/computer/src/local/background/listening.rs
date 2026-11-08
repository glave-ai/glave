//! Which TCP ports the processes of this machine's commands in the background listen on, on
//! loopback or on every address, so this machine's own loopback reaches each: where a dev server
//! a command started serves (`specs/computers.md`, "Background commands"). A command's processes
//! are those in its process group and everything they started, followed by parent as the
//! sandbox's own session on Linux leaves the group; a process that left on purpose (a daemon) is
//! not followed, as stopping one does not follow it. On Linux it is read from `/proc`; on macOS
//! from `lsof`, the system's own, by its path; elsewhere nothing is found.

use std::collections::{BTreeMap, BTreeSet};
#[cfg(any(target_os = "linux", test))]
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::time::Duration;

use glave_ports::host::is_loopback;
use glave_process::Processes;

/// How long a look may take: a few local reads, or one run of `lsof`.
const LOOKING: Duration = Duration::from_secs(5);

/// The ports each of `groups`, a process group by its leader's id, listens on, lowest first; a
/// group listening on none is left out.
pub(super) async fn ports(groups: &[u32], processes: &Processes) -> BTreeMap<u32, Vec<u16>> {
    if groups.is_empty() {
        return BTreeMap::new();
    }
    let found = tokio::time::timeout(LOOKING, found(groups, processes))
        .await
        .unwrap_or_default();
    found
        .into_iter()
        .map(|(group, ports)| (group, ports.into_iter().collect()))
        .collect()
}

#[cfg(target_os = "linux")]
async fn found(groups: &[u32], _: &Processes) -> BTreeMap<u32, BTreeSet<u16>> {
    let groups = groups.to_vec();
    tokio::task::spawn_blocking(move || proc::found(&groups))
        .await
        .unwrap_or_default()
}

/// The only `lsof` trusted: one earlier on `PATH` could be planted.
#[cfg(target_os = "macos")]
const LSOF: &str = "/usr/sbin/lsof";

#[cfg(target_os = "macos")]
async fn found(groups: &[u32], processes: &Processes) -> BTreeMap<u32, BTreeSet<u16>> {
    let groups: Vec<String> = groups.iter().map(u32::to_string).collect();
    let mut lsof = glave_process::Program::new(LSOF);
    // Numbers, not names; the groups' TCP sockets that listen; the group and the address.
    lsof.args(["-nP", "-a", "-g"])
        .arg(groups.join(","))
        .args(["-iTCP", "-sTCP:LISTEN", "-F", "gn"])
        .no_input();
    // It ends with 1 when it found nothing: what it printed says what it found either way.
    match processes.output(&mut lsof, LOOKING).await {
        Ok(output) => in_lsof(&String::from_utf8_lossy(&output.stdout)),
        Err(error) => {
            tracing::debug!(%error, "could not look for the ports commands listen on");
            BTreeMap::new()
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
async fn found(_: &[u32], _: &Processes) -> BTreeMap<u32, BTreeSet<u16>> {
    BTreeMap::new()
}

/// Whether a socket listening at `address` is reached at this machine's loopback.
fn reached_here(address: IpAddr) -> bool {
    address.is_unspecified() || is_loopback(address)
}

/// What `lsof -F gn` printed: each group's listening ports, from its `g` lines and the `n` lines
/// after them (`127.0.0.1:5173`, `[::1]:5173`, `*:5173` for every address).
#[cfg(any(target_os = "macos", test))]
fn in_lsof(printed: &str) -> BTreeMap<u32, BTreeSet<u16>> {
    let mut found: BTreeMap<u32, BTreeSet<u16>> = BTreeMap::new();
    let mut group = None;
    for line in printed.lines() {
        if let Some(id) = line.strip_prefix('g') {
            group = id.parse().ok();
        } else if let (Some(name), Some(group)) = (line.strip_prefix('n'), group)
            && let Some((host, port)) = name.rsplit_once(':')
            && let Ok(port) = port.parse::<u16>()
        {
            let bare = host.trim_start_matches('[').trim_end_matches(']');
            if host == "*" || bare.parse().is_ok_and(reached_here) {
                found.entry(group).or_default().insert(port);
            }
        }
    }
    found
}

/// A process, as `/proc/<pid>/stat` names it: its parent and its group.
#[cfg(any(target_os = "linux", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Process {
    parent: u32,
    group: u32,
}

/// The parent and group in what `/proc/<pid>/stat` holds: `pid (name) state ppid pgrp …`, the
/// name in parentheses that may itself hold any of them, so read from the last.
#[cfg(any(target_os = "linux", test))]
fn in_stat(stat: &str) -> Option<Process> {
    let (_, after) = stat.rsplit_once(')')?;
    let mut fields = after.split_whitespace().skip(1);
    let parent = fields.next()?.parse().ok()?;
    let group = fields.next()?.parse().ok()?;
    Some(Process { parent, group })
}

/// The deepest a command's processes are followed from its group, parent by parent.
#[cfg(any(target_os = "linux", test))]
const DEEPEST: usize = 64;

/// Which of `groups` each of `processes` belongs to: its own group, or the first group of a
/// process above it.
#[cfg(any(target_os = "linux", test))]
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

/// The sockets listening in a table of `/proc/net/tcp` or `tcp6`, each by its inode, with its
/// port, where this machine's loopback reaches it. An address is printed as the words of its
/// bytes in network order, each read as this machine reads a number; the port as a number.
#[cfg(any(target_os = "linux", test))]
fn in_table(table: &str) -> Vec<(u64, u16)> {
    /// A socket that listens, as the table writes its state.
    const LISTEN: &str = "0A";
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let (local, state, inode) = (fields.get(1)?, fields.get(3)?, fields.get(9)?);
            if *state != LISTEN {
                return None;
            }
            let (address, port) = local.split_once(':')?;
            let address = in_hex(address)?;
            let port = u16::from_str_radix(port, 16).ok()?;
            reached_here(address).then_some((inode.parse().ok()?, port))
        })
        .collect()
}

/// An address as `/proc/net/tcp` writes it: one word in hex for IPv4, four for IPv6.
#[cfg(any(target_os = "linux", test))]
fn in_hex(hex: &str) -> Option<IpAddr> {
    let mut bytes = Vec::with_capacity(16);
    for word in 0..hex.len() / 8 {
        let digits = hex.get(word * 8..word * 8 + 8)?;
        bytes.extend(u32::from_str_radix(digits, 16).ok()?.to_ne_bytes());
    }
    match hex.len() {
        8 => <[u8; 4]>::try_from(bytes).ok().map(IpAddr::from),
        32 => <[u8; 16]>::try_from(bytes).ok().map(IpAddr::from),
        _ => None,
    }
}

/// The inode in a link of `/proc/<pid>/fd` to a socket: `socket:[12345]`.
#[cfg(any(target_os = "linux", test))]
fn socket_inode(link: &str) -> Option<u64> {
    link.strip_prefix("socket:[")?.strip_suffix(']')?.parse().ok()
}

/// Reading `/proc`, on this machine's own network namespace: a sandboxed command's socket in
/// a namespace of its own (the sandbox without the network) is in none of its tables, and no
/// one here could reach it.
#[cfg(target_os = "linux")]
mod proc {
    use std::collections::{BTreeMap, BTreeSet, HashMap};
    use std::fs;

    use super::{Process, in_stat, in_table, owners, socket_inode};

    /// The most processes, and the most open files of each, looked at.
    const MOST: usize = 65_536;

    pub(super) fn found(groups: &[u32]) -> BTreeMap<u32, BTreeSet<u16>> {
        let owned = owners(&processes(), groups);
        let mut sockets: HashMap<u64, u32> = HashMap::new();
        #[expect(
            clippy::iter_over_hash_type,
            reason = "each socket is noted by its inode, whatever the order"
        )]
        for (pid, group) in &owned {
            let Ok(files) = fs::read_dir(format!("/proc/{pid}/fd")) else {
                continue;
            };
            for file in files.flatten().take(MOST) {
                let link = fs::read_link(file.path()).ok();
                if let Some(inode) = link.as_ref().and_then(|link| socket_inode(&link.to_string_lossy())) {
                    sockets.insert(inode, *group);
                }
            }
        }
        let mut found: BTreeMap<u32, BTreeSet<u16>> = BTreeMap::new();
        for table in ["/proc/self/net/tcp", "/proc/self/net/tcp6"] {
            let Ok(text) = fs::read_to_string(table) else {
                continue;
            };
            for (inode, port) in in_table(&text) {
                if let Some(group) = sockets.get(&inode) {
                    found.entry(*group).or_default().insert(port);
                }
            }
        }
        found
    }

    /// Every process in sight, by its id.
    fn processes() -> HashMap<u32, Process> {
        let Ok(entries) = fs::read_dir("/proc") else {
            return HashMap::new();
        };
        entries
            .flatten()
            .take(MOST)
            .filter_map(|entry| {
                let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
                let stat = fs::read_to_string(entry.path().join("stat")).ok()?;
                Some((pid, in_stat(&stat)?))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
