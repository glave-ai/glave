//! How long a Unix socket's path may be, which every socket glave and its tests make keeps to.

/// The longest path for a Unix socket that every system glave runs on takes, in bytes: macOS
/// holds 104 with the terminator, Linux 108. A socket whose usual path is longer goes in a
/// short folder of `/tmp` instead.
pub const MAX_SOCKET_PATH: usize = 100;
