//! Where commands listen on Linux, read from `/proc` on this machine's own network namespace: a
//! sandboxed command's socket in a namespace of its own (the sandbox without the network) is in
//! none of its tables, and no one here could reach it.

use std::net::{IpAddr, SocketAddr};

use super::{Process, reached_here};

/// The parent and group in what `/proc/<pid>/stat` holds: `pid (name) state ppid pgrp …`, the
/// name in parentheses that may itself hold any of them, so read from the last.
pub(super) fn in_stat(stat: &str) -> Option<Process> {
    let (_, after) = stat.rsplit_once(')')?;
    let mut fields = after.split_whitespace().skip(1);
    let parent = fields.next()?.parse().ok()?;
    let group = fields.next()?.parse().ok()?;
    Some(Process { parent, group })
}

/// The sockets listening in a table of `/proc/net/tcp` or `tcp6`, each by its inode, with its
/// address, where this machine's loopback reaches it. An address is printed as the words of
/// its bytes in network order, each read as this machine reads a number; the port as a number.
pub(super) fn in_table(table: &str) -> Vec<(u64, SocketAddr)> {
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
            let at = SocketAddr::new(address, port);
            reached_here(address).then_some((inode.parse().ok()?, at))
        })
        .collect()
}

/// An address as `/proc/net/tcp` writes it: one word in hex for IPv4, four for IPv6.
pub(super) fn in_hex(hex: &str) -> Option<IpAddr> {
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
pub(super) fn socket_inode(link: &str) -> Option<u64> {
    link.strip_prefix("socket:[")?
        .strip_suffix(']')?
        .parse()
        .ok()
}

#[cfg(target_os = "linux")]
pub(super) use walk::found;

/// The walk itself, through every process and its open files, stopping at its deadline.
#[cfg(target_os = "linux")]
mod walk {
    use std::collections::HashMap;
    use std::fs;
    use std::time::Instant;

    use super::super::{Found, Process, owners};
    use super::{in_stat, in_table, socket_inode};

    /// The most processes, and the most open files of each, looked at.
    const MOST: usize = 65_536;

    /// Where each of `groups` listens, as far as the walk got by `until`.
    pub(in super::super) fn found(groups: &[u32], until: Instant) -> Found {
        let in_time = |_: &_| Instant::now() < until;
        let owned = owners(&processes(until), groups);
        let mut sockets: HashMap<u64, u32> = HashMap::new();
        #[expect(
            clippy::iter_over_hash_type,
            reason = "each socket is noted by its inode, whatever the order"
        )]
        for (pid, group) in &owned {
            let Ok(files) = fs::read_dir(format!("/proc/{pid}/fd")) else {
                continue;
            };
            for file in files.flatten().take(MOST).take_while(in_time) {
                let link = fs::read_link(file.path()).ok();
                if let Some(inode) = link
                    .as_ref()
                    .and_then(|link| socket_inode(&link.to_string_lossy()))
                {
                    sockets.insert(inode, *group);
                }
            }
        }
        let mut found = Found::new();
        for table in ["/proc/self/net/tcp", "/proc/self/net/tcp6"] {
            let Ok(text) = fs::read_to_string(table) else {
                continue;
            };
            for (inode, at) in in_table(&text) {
                if let Some(group) = sockets.get(&inode) {
                    found
                        .entry(*group)
                        .or_default()
                        .insert((at.port(), at.ip()));
                }
            }
        }
        found
    }

    /// Every process in sight by `until`, by its id.
    fn processes(until: Instant) -> HashMap<u32, Process> {
        let Ok(entries) = fs::read_dir("/proc") else {
            return HashMap::new();
        };
        entries
            .flatten()
            .take(MOST)
            .take_while(|_| Instant::now() < until)
            .filter_map(|entry| {
                let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
                let stat = fs::read_to_string(entry.path().join("stat")).ok()?;
                Some((pid, in_stat(&stat)?))
            })
            .collect()
    }
}
