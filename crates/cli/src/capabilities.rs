//! The capabilities a service's unit gives glave, such as the right to listen on port 443
//! (`specs/one-server.md`, "HTTPS of its own"), stay glave's own: the programs it starts get
//! none. On Linux a capability a unit grants is ambient, which every program started after
//! inherits; bubblewrap refuses to start with one it was not given as setuid, and no command
//! needs one. So glave clears its ambient set first thing, before it makes a thread (each
//! thread has its own), and keeps for itself what it was given.

/// Clears the ambient capabilities this process would hand every program it starts. A kernel
/// without them (before Linux 4.3) has none to clear.
#[cfg(target_os = "linux")]
pub(crate) fn keep_to_itself() {
    if let Err(error) = rustix::thread::clear_ambient_capability_set()
        && error != rustix::io::Errno::INVAL
    {
        eprintln!(
            "glave: warning: the programs glave starts may inherit its capabilities: {error}"
        );
    }
}

/// Nothing to clear: only Linux hands capabilities on this way.
#[cfg(not(target_os = "linux"))]
pub(crate) fn keep_to_itself() {}
