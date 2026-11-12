//! glave joining a thread's network namespace for one of its commands (`namespace.rs`):
//! started as `glave __glave-enter <pid> <user> <network> -- <command>…`, it joins the user and
//! network namespaces of the process `pid`, the forwarder that holds them, each only if it is
//! the one named (by its inode, so a number given to another process since is never followed),
//! then becomes the command (`exec`): bubblewrap, which starts the command in that network
//! rather than a new one. Joining a user namespace takes a process with one thread, so this
//! runs first thing in `main`.

use std::ffi::OsString;
use std::fs::File;
use std::os::fd::AsFd as _;
use std::os::unix::fs::MetadataExt as _;

use rustix::thread::{LinkNameSpaceType, move_into_link_name_space};

const USAGE: &str = "usage: glave __glave-enter <pid> <user> <network> -- <command>…";

/// Runs it on its arguments (after `__glave-enter`). Returns only on failure, saying why.
pub(in crate::local::sandbox) fn enter(args: &[OsString]) -> String {
    let [pid, user, network, separator, program, rest @ ..] = args else {
        return USAGE.into();
    };
    if separator != "--" {
        return USAGE.into();
    }
    let Some(pid) = number(pid) else {
        return USAGE.into();
    };
    // Both opened before either is joined: from inside the user namespace, the holder's
    // namespaces may no longer be this process's to open.
    let (user, network) = match (named(pid, "user", user), named(pid, "net", network)) {
        (Ok(user), Ok(network)) => (user, network),
        (Err(why), _) | (_, Err(why)) => return why,
    };
    let joined =
        move_into_link_name_space(user.as_fd(), Some(LinkNameSpaceType::User)).and_then(|()| {
            move_into_link_name_space(network.as_fd(), Some(LinkNameSpaceType::Network))
        });
    if let Err(error) = joined {
        return format!("could not join the thread's network: {error}");
    }
    // Opened to close on `exec`, so the command holds neither.
    glave_process::Program::new(program)
        .args(rest)
        .exec()
        .to_string()
}

/// A number given as an argument.
fn number<T: std::str::FromStr>(arg: &OsString) -> Option<T> {
    arg.to_str()?.parse().ok()
}

/// The namespace of `kind` that process `pid` is in, if it is still the one whose inode is
/// `inode`; otherwise why not.
fn named(pid: u32, kind: &str, inode: &OsString) -> Result<File, String> {
    let gone = || "the thread's network has ended; its next command starts a new one".to_owned();
    let file = File::open(format!("/proc/{pid}/ns/{kind}")).map_err(|_| gone())?;
    let found = file.metadata().map_err(|_| gone())?.ino();
    if number::<u64>(inode) != Some(found) {
        return Err(gone());
    }
    Ok(file)
}
