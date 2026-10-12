//! Finding bubblewrap, checking that it can start here, and its arguments for one command.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::mounts::{Layout, Mount};

/// Runs bubblewrap with the seccomp filter on descriptor 3, read from standard input, and
/// standard input then empty: `sh -c <this> <bwrap> <args>…`.
const SHELL: &str = r#"exec "$0" "$@" 3<&0 </dev/null"#;

/// Where the wrapper is: `/bin/sh` is on every Linux.
pub(super) const SH: &str = "/bin/sh";

/// A new session (so a command cannot push keystrokes into the person's terminal), killed
/// with its parent, in namespaces of its own; with no new privileges, which bubblewrap sets,
/// and no capabilities. bubblewrap keeps them all for a command that is root inside, as it is
/// when glave runs as root (containers, cloud machines): root could then read through the
/// unreadable stand-ins over hidden files and see them empty instead of refused.
const ISOLATION: &[&str] = &[
    "--new-session",
    "--die-with-parent",
    "--unshare-user",
    "--unshare-pid",
    "--unshare-ipc",
    "--cap-drop",
    "ALL",
];

/// The first `bwrap` on `PATH` that a command could not have planted: a real file outside
/// the temp folders, the current folder and `writable`, the folders commands may write.
pub(super) fn find(writable: &[&Path]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let mut unsafe_places: Vec<PathBuf> = [std::env::temp_dir(), PathBuf::from("/tmp")]
        .into_iter()
        .chain(std::env::current_dir().ok())
        .filter_map(|place| std::fs::canonicalize(place).ok())
        .collect();
    unsafe_places.extend(writable.iter().map(|place| place.to_path_buf()));
    std::env::split_paths(&path)
        .filter(|folder| folder.is_absolute())
        .filter_map(|folder| std::fs::canonicalize(folder.join("bwrap")).ok())
        .find(|real| real.is_file() && !unsafe_places.iter().any(|place| real.starts_with(place)))
}

/// Whether `bwrap` can make the sandbox here: it cannot where unprivileged user namespaces
/// are off, or where AppArmor stops it making them (Ubuntu 23.10 and later, without a
/// profile for it). Blocking; says why not.
pub(super) fn starts(bwrap: &Path) -> Result<(), String> {
    let output = Command::new(bwrap)
        .args(ISOLATION)
        .args(["--unshare-net", "--ro-bind", "/", "/", "--dev", "/dev"])
        .args(["--proc", "/proc", "--", "true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&output.stderr);
    Err(said.trim().chars().take(300).collect())
}

/// The arguments for `sh` that start one command in the sandbox, up to the command itself.
pub(super) fn args(bwrap: &Path, layout: &Layout, mounts: &[Mount]) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["-c".into(), SHELL.into(), bwrap.into()];
    args.extend(ISOLATION.iter().map(OsString::from));
    if !layout.network {
        args.push("--unshare-net".into());
    }
    // The whole machine read-only, with fresh devices and processes of its own: the host's
    // `/proc` would show other processes' environments.
    for arg in ["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc"] {
        args.push(arg.into());
    }
    for mount in mounts {
        let (how, from, to) = match mount {
            Mount::Writable(path) => ("--bind", path, path),
            Mount::ReadOnly(path) => ("--ro-bind", path, path),
            Mount::HiddenFile(path) => ("--ro-bind", &layout.empty_file, path),
            Mount::HiddenFolder(path) => ("--ro-bind", &layout.empty_folder, path),
        };
        args.extend([how.into(), from.into(), to.into()]);
    }
    args.extend(["--chdir".into(), layout.root.clone().into_os_string()]);
    args.extend(["--seccomp", "3", "--"].map(OsString::from));
    args
}
