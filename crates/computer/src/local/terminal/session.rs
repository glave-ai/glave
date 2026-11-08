//! Everything a terminal's shell started, found as the operating system groups it: its session.
//! An interactive shell puts each job in a process group of its own, so stopping the shell's
//! group leaves its jobs; its session holds them all. Linux names a process's session in
//! `/proc`, which outlives the shell; macOS names none to a program, so there a process is
//! found by the terminal it runs in, which only the shell's own exit takes from it.

use std::time::Duration;

use rustix::process::{Pid, Signal, kill_process, kill_process_group};

/// How long what a hang-up was sent to has to end before it is killed.
const GRACE: Duration = Duration::from_secs(1);

/// How often what is left is looked for meanwhile.
const LOOK_EVERY: Duration = Duration::from_millis(50);

/// A terminal's session: the process that leads it, and on macOS the terminal it runs in, by
/// which its processes are found there.
#[derive(Debug, Clone)]
pub(super) struct Session {
    leader: Pid,
    /// The terminal's name, as `/dev/ttys012`.
    #[cfg(target_os = "macos")]
    tty: String,
}

impl Session {
    /// The session `leader` leads, in the terminal named `tty`, as `/dev/ttys012`, by which
    /// macOS finds its processes.
    #[cfg(target_os = "macos")]
    pub(super) fn new(leader: Pid, tty: String) -> Self {
        Self { leader, tty }
    }

    /// The session `leader` leads, whose processes the system names.
    #[cfg(not(target_os = "macos"))]
    pub(super) fn new(leader: Pid) -> Self {
        Self { leader }
    }

    /// The process that leads it.
    pub(super) fn leader(&self) -> Pid {
        self.leader
    }

    /// Hangs everything in it up, as a terminal's hang-up does, in the order macOS needs: what
    /// its leader started first, while the leader still holds the terminal they are found by
    /// (`end_others`, off the async threads), then the leader's group, its leader once reaped
    /// left alone.
    pub(super) async fn hang_up(&self, leader_reaped: bool) {
        let session = self.clone();
        let _ = tokio::task::spawn_blocking(move || session.end_others()).await;
        if !leader_reaped {
            let _ = kill_process_group(self.leader, Signal::HUP);
        }
    }

    /// Ends what is in it but its leader, which asks, blocking: SIGHUP to each, then SIGKILL to
    /// what is left a second later, as it is found then.
    pub(super) fn end_others(&self) {
        let others = || {
            let mut members = self.members_now();
            members.retain(|member| *member != self.leader);
            members
        };
        for member in others() {
            let _ = kill_process(member, Signal::HUP);
        }
        let began = std::time::Instant::now();
        while began.elapsed() < GRACE && !others().is_empty() {
            std::thread::sleep(LOOK_EVERY);
        }
        for member in others() {
            let _ = kill_process(member, Signal::KILL);
        }
    }

    /// The processes in it now; blocking.
    fn members_now(&self) -> Vec<Pid> {
        platform(self)
    }
}

#[cfg(target_os = "linux")]
fn platform(session: &Session) -> Vec<Pid> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let wanted = session.leader.as_raw_nonzero().get();
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str()?.parse::<i32>().ok())
        .filter(|&pid| {
            std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .ok()
                .and_then(|stat| session_of(&stat))
                == Some(wanted)
        })
        .filter_map(Pid::from_raw)
        .collect()
}

/// The session (field 6) in a `/proc/<pid>/stat` line. The name before it is in parentheses
/// and may hold anything, so the fields are counted after its last `)`.
#[cfg(any(target_os = "linux", test))]
fn session_of(stat: &str) -> Option<i32> {
    let (_, fields) = stat.rsplit_once(')')?;
    fields.split_whitespace().nth(3)?.parse().ok()
}

/// How long `ps` has to list this computer's processes, which it does in milliseconds.
#[cfg(target_os = "macos")]
const PS_ANSWERS_WITHIN: Duration = Duration::from_secs(5);

#[cfg(target_os = "macos")]
fn platform(session: &Session) -> Vec<Pid> {
    let tty = session.tty.trim_start_matches("/dev/");
    let mut ps = glave_process::Program::new("/bin/ps");
    ps.args(["-A", "-o", "pid=,tty="])
        .env_clear()
        .env("LC_ALL", "C")
        .no_input();
    let Ok(listed) = ps.output_blocking(PS_ANSWERS_WITHIN) else {
        return Vec::new();
    };
    String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter_map(|line| {
            let (pid, on) = line.trim().split_once(char::is_whitespace)?;
            (on.trim() == tty).then(|| pid.parse().ok()).flatten()
        })
        .filter_map(Pid::from_raw)
        .collect()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn platform(_session: &Session) -> Vec<Pid> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_is_read_after_the_name_whatever_it_holds() {
        let stat = "42 (a) b (c) S 1 42 40 0 -1 4194560 1 0 0 0 0 0 0 0 20 0 1 0 98765 1 2";
        assert_eq!(session_of(stat), Some(40));
        assert_eq!(session_of("42 (cut"), None);
    }
}
