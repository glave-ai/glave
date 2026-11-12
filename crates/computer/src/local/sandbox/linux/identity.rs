//! A process named by its number and its start, and the user and network namespaces it is in,
//! all read through one handle on its folder in `/proc`: a folder opened for a process that has
//! since ended reads nothing, so what is read is that process's or nothing, and a number given
//! to another process since names one that started later. glave records the processes that
//! hold a thread's network (`namespace.rs`), and a command joins it only through them while
//! they are still the processes recorded (`enter.rs`).

use std::fs::File;
use std::io::{self, Read as _};
use std::os::fd::OwnedFd;
use std::os::unix::fs::MetadataExt as _;

use rustix::fs::{CWD, Mode, OFlags, openat};

/// A process as glave records it: its number, its start (in the system's clock ticks since it
/// started), and the inodes of its user and network namespaces, which name them while they last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Process {
    pub(super) pid: u32,
    pub(super) started: u64,
    pub(super) user: u64,
    pub(super) network: u64,
}

/// A process's user and network namespaces, open.
#[derive(Debug)]
pub(super) struct Namespaces {
    pub(super) user: File,
    pub(super) network: File,
}

impl Process {
    /// Process `pid` as it is now, and its namespaces, open.
    pub(super) fn of(pid: u32) -> io::Result<(Self, Namespaces)> {
        let flags = OFlags::RDONLY | OFlags::CLOEXEC;
        let folder = openat(
            CWD,
            format!("/proc/{pid}"),
            flags | OFlags::DIRECTORY,
            Mode::empty(),
        )?;
        let started = started(&folder)?;
        let opened = |name: &str| openat(&folder, name, flags, Mode::empty()).map(File::from);
        let namespaces = Namespaces {
            user: opened("ns/user")?,
            network: opened("ns/net")?,
        };
        let process = Self {
            pid,
            started,
            user: namespaces.user.metadata()?.ino(),
            network: namespaces.network.metadata()?.ino(),
        };
        Ok((process, namespaces))
    }

    /// Its namespaces, open, while it is still this process in them.
    pub(super) fn namespaces(&self) -> Option<Namespaces> {
        let (now, namespaces) = Self::of(self.pid).ok()?;
        (now == *self).then_some(namespaces)
    }

    /// Itself as arguments, which [`Process::from_args`] reads back.
    pub(super) fn args(&self) -> [String; 4] {
        let pid = u64::from(self.pid);
        [pid, self.started, self.user, self.network].map(|number| number.to_string())
    }

    /// The process `args` name, as [`Process::args`] wrote it.
    pub(super) fn from_args(args: &[std::ffi::OsString]) -> Option<Self> {
        let [pid, started, user, network] = args else {
            return None;
        };
        let number = |arg: &std::ffi::OsString| arg.to_str()?.parse::<u64>().ok();
        Some(Self {
            pid: u32::try_from(number(pid)?).ok()?,
            started: number(started)?,
            user: number(user)?,
            network: number(network)?,
        })
    }
}

/// When the process whose `/proc` folder is `folder` started: the 22nd field of its `stat`,
/// counted after its name, which may hold spaces and parentheses itself.
fn started(folder: &OwnedFd) -> io::Result<u64> {
    let stat = openat(
        folder,
        "stat",
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let mut text = String::new();
    File::from(stat).read_to_string(&mut text)?;
    text.rsplit_once(')')
        .and_then(|(_, after)| after.split_whitespace().nth(19))
        .and_then(|started| started.parse().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "an unreadable stat"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_is_itself_and_another_number_is_not() {
        let (this, _) = Process::of(std::process::id()).unwrap();
        assert!(this.namespaces().is_some());
        let read = Process::from_args(&this.args().map(Into::into)).unwrap();
        assert_eq!(read, this);
        let later = Process {
            started: this.started + 1,
            ..this
        };
        assert!(
            later.namespaces().is_none(),
            "another start is another process"
        );
    }
}
