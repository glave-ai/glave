//! Where commands listen on macOS: every process's parent and group from the system's `ps`, the
//! commands' processes followed from their groups by parent as on Linux, then their listening
//! sockets from the system's `lsof`, each by its path, since one earlier on `PATH` could be
//! planted.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use super::{Found, Process, reached_here};

/// What `ps -A -o pid=,ppid=,pgid=` printed: each process's parent and group, by its id.
pub(super) fn in_ps(printed: &str) -> HashMap<u32, Process> {
    printed
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace().map(str::parse::<u32>);
            let (Some(Ok(pid)), Some(Ok(parent)), Some(Ok(group))) =
                (fields.next(), fields.next(), fields.next())
            else {
                return None;
            };
            Some((pid, Process { parent, group }))
        })
        .collect()
}

/// What `lsof -F ptn` printed, of the processes `owned` names with the group each belongs to:
/// each group's listening addresses, from each process's `p` line and, after it, each socket's
/// family (`tIPv4`, `tIPv6`) and address (`n127.0.0.1:5173`, `n[::1]:5173`, `n*:5173` for every
/// address of its family).
pub(super) fn in_lsof(printed: &str, owned: &HashMap<u32, u32>) -> Found {
    let mut found = Found::new();
    let (mut group, mut six) = (None, false);
    for line in printed.lines() {
        if let Some(pid) = line.strip_prefix('p') {
            group = pid
                .parse::<u32>()
                .ok()
                .and_then(|pid| owned.get(&pid).copied());
        } else if let Some(family) = line.strip_prefix('t') {
            six = family == "IPv6";
        } else if let (Some(name), Some(group)) = (line.strip_prefix('n'), group)
            && let Some((host, port)) = name.rsplit_once(':')
            && let Ok(port) = port.parse::<u16>()
        {
            let every = if six {
                IpAddr::V6(Ipv6Addr::UNSPECIFIED)
            } else {
                IpAddr::V4(Ipv4Addr::UNSPECIFIED)
            };
            let bare = host.trim_start_matches('[').trim_end_matches(']');
            let address = if host == "*" {
                Some(every)
            } else {
                bare.parse().ok()
            };
            if let Some(address) = address.filter(|address| reached_here(*address)) {
                found.entry(group).or_default().insert((port, address));
            }
        }
    }
    found
}

/// The whole lines of `printed`, a program's output kept to its first `most` bytes and one
/// more: past that, its last line was cut short, and is left out rather than read as another.
pub(super) fn whole_lines(printed: &[u8], most: usize) -> String {
    let text = String::from_utf8_lossy(printed);
    if printed.len() <= most {
        return text.into_owned();
    }
    text.rsplit_once('\n')
        .map(|(whole, _)| format!("{whole}\n"))
        .unwrap_or_default()
}

#[cfg(target_os = "macos")]
pub(super) use run::found;

/// Running `ps` and `lsof`, each within its limit, its output capped.
#[cfg(target_os = "macos")]
mod run {
    use glave_process::{Processes, Program};

    use super::super::{Found, LOOKING, owners};
    use super::{in_lsof, in_ps, whole_lines};

    const PS: &str = "/bin/ps";
    const LSOF: &str = "/usr/sbin/lsof";

    /// The most of what either program prints that is read: far more than a computer's
    /// processes and a workspace's sockets take. Past it, the program is stopped and its last
    /// line left out.
    const MOST_PRINTED: usize = 1024 * 1024;

    /// Where each of `groups` listens, as `ps` and `lsof` say within their limits.
    pub(in super::super) async fn found(groups: &[u32], processes: &Processes) -> Found {
        let mut ps = Program::new(PS);
        ps.args(["-A", "-o", "pid=,ppid=,pgid="]).no_input();
        let Some(listed) = printed(processes, &mut ps).await else {
            return Found::new();
        };
        let owned = owners(&in_ps(&listed), groups);
        if owned.is_empty() {
            return Found::new();
        }
        let mut pids: Vec<u32> = owned.keys().copied().collect();
        pids.sort_unstable();
        let pids: Vec<String> = pids.iter().map(u32::to_string).collect();
        let mut lsof = Program::new(LSOF);
        // Numbers, not names; those processes' TCP sockets that listen; each one's process,
        // family and address.
        lsof.args(["-nP", "-a", "-p"])
            .arg(pids.join(","))
            .args(["-iTCP", "-sTCP:LISTEN", "-F", "ptn"])
            .no_input();
        // It ends with 1 when it found nothing: what it printed says what it found either way.
        printed(processes, &mut lsof)
            .await
            .map(|sockets| in_lsof(&sockets, &owned))
            .unwrap_or_default()
    }

    /// What `program` printed, its whole lines, within `LOOKING`; none when it could not run.
    async fn printed(processes: &Processes, program: &mut Program) -> Option<String> {
        match processes
            .output_at_most(program, LOOKING, MOST_PRINTED)
            .await
        {
            Ok(output) => Some(whole_lines(&output.stdout, MOST_PRINTED)),
            Err(error) => {
                tracing::debug!(%error, "could not look where commands listen");
                None
            }
        }
    }
}
