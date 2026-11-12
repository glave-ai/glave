//! glave joining a thread's network namespace (`namespace.rs`): started as `glave __glave-enter
//! <pid> <started> <user> <network> -- <program>…`, it joins the user and network namespaces of
//! the process that holds them, only while it is still the process recorded (`identity.rs`), so
//! a number given to another process since is never followed, then becomes the program
//! (`exec`): for a command, bubblewrap, which starts it in that network rather than a new one;
//! for the forwarder, glave again. Joining a user namespace takes a process with one thread, so
//! this runs first thing in `main`.

use std::ffi::OsString;
use std::os::fd::AsFd as _;

use rustix::thread::{LinkNameSpaceType, move_into_link_name_space};

use super::identity::Process;

const USAGE: &str = "usage: glave __glave-enter <pid> <started> <user> <network> -- <program>…";

/// Runs it on its arguments (after `__glave-enter`). Returns only on failure, saying why.
pub(in crate::local::sandbox) fn enter(args: &[OsString]) -> String {
    let (Some(named), Some([separator, program, rest @ ..])) = (args.get(..4), args.get(4..))
    else {
        return USAGE.into();
    };
    let Some(holder) = Process::from_args(named).filter(|_| separator == "--") else {
        return USAGE.into();
    };
    // Both opened before either is joined: from inside the user namespace, the holder's may no
    // longer be this process's to open. Opened to close on `exec`, so the program holds neither.
    let Some(namespaces) = holder.namespaces() else {
        return "the thread's network has ended; its next command starts a new one".into();
    };
    let joined = move_into_link_name_space(namespaces.user.as_fd(), Some(LinkNameSpaceType::User))
        .and_then(|()| {
            move_into_link_name_space(namespaces.network.as_fd(), Some(LinkNameSpaceType::Network))
        });
    if let Err(error) = joined {
        return format!("could not join the thread's network: {error}");
    }
    glave_process::Program::new(program)
        .args(rest)
        .exec()
        .to_string()
}
