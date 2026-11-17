//! The capabilities a service's unit gives glave, such as the right to listen on port 443
//! (`specs/one-server.md`, "HTTPS of its own"), stay glave's own: the programs it starts get
//! none. On Linux a capability a unit grants is ambient, which every program started after
//! inherits; bubblewrap refuses to start with one it was not given as setuid, and no command
//! needs one. So glave clears its ambient set first thing, before it makes a thread (each
//! thread has its own), and keeps for itself what it was given.

use crate::said::Said;

/// Clears the ambient capabilities this process would hand every program it starts, and why it
/// could not, which glave says ([`warn`]) once it knows where it says things. A kernel without
/// them (before Linux 4.3) has none to clear.
#[cfg(target_os = "linux")]
pub(crate) fn keep_to_itself() -> Option<String> {
    match rustix::thread::clear_ambient_capability_set() {
        Err(error) if error != rustix::io::Errno::INVAL => Some(error.to_string()),
        _ => None,
    }
}

/// Nothing to clear: only Linux hands capabilities on this way.
#[cfg(not(target_os = "linux"))]
pub(crate) fn keep_to_itself() -> Option<String> {
    None
}

/// Warns, as `said`, that the capabilities could not be kept, for `error`.
pub(crate) fn warn(said: Said, error: &str) {
    said.warned(
        "the programs glave starts may inherit its capabilities",
        &error,
    );
}
