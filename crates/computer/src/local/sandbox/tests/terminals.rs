//! Other terminals, under the real sandbox: a command opens no terminal's device but its own
//! (`specs/invariants.md`, 33).

use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStringExt as _;

use rustix::fs::{Mode, OFlags};

use super::*;

/// A terminal of another session's, as the person's Terminal tab is: a pseudo-terminal this
/// test holds open, and the device its shell would read what is typed from.
pub(in crate::local::sandbox) struct AnotherTerminal {
    _held: OwnedFd,
    pub(in crate::local::sandbox) path: PathBuf,
}

impl AnotherTerminal {
    pub(in crate::local::sandbox) fn open() -> Self {
        let held = rustix::fs::open(
            "/dev/ptmx",
            OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        rustix::pty::grantpt(&held).unwrap();
        rustix::pty::unlockpt(&held).unwrap();
        let name = rustix::pty::ptsname(&held, Vec::new()).unwrap();
        let path = PathBuf::from(OsString::from_vec(name.into_bytes()));
        Self { _held: held, path }
    }

    /// A script that says each way it opened this terminal: `read-opened`, `write-opened`.
    pub(in crate::local::sandbox) fn opens(&self) -> String {
        format!(
            "(exec 3< '{0}') && echo read-opened; (exec 3> '{0}') && echo write-opened; true",
            self.path.display()
        )
    }
}

/// A terminal the command makes itself, whose name `tty` says inside it.
#[cfg(target_os = "macos")]
const OWN_TERMINAL: &str = "script -q /dev/null tty";
#[cfg(not(target_os = "macos"))]
const OWN_TERMINAL: &str = "script -qec tty /dev/null";

/// Another session's terminal can't be opened from the sandbox, to read or to write, though
/// outside it the same script opens it both ways; what a command does with terminals of its
/// own still works: `tty` says it has none, and one it makes itself runs a program.
#[test]
fn a_sandboxed_command_opens_no_other_terminal_but_makes_its_own() {
    if !enforced() {
        return;
    }
    let dir = scratch("sandbox-terminals");
    let (computers, sandbox) = computers(&dir);
    let other = AnotherTerminal::open();
    let script = format!(
        "{}; tty; git --version >/dev/null && echo git-works; \
         {OWN_TERMINAL} | tr -d '\\r' | grep -E '/dev/(ttys|pts/)[0-9]' && echo own-terminal-works",
        other.opens()
    );
    block_on!(async {
        let here = open_app(&computers, &dir).await;
        let (ending, said) = run(&computers, here, &sandbox, &script).await;
        assert_eq!(ending, Ending::Exited { code: 0 }, "{said}");
        assert!(!said.contains("read-opened"), "{said}");
        assert!(!said.contains("write-opened"), "{said}");
        assert!(said.contains("not a tty"), "{said}");
        assert!(said.contains("git-works"), "{said}");
        assert!(said.contains("own-terminal-works"), "{said}");
    });
    // Control: outside the sandbox the same script opens it both ways.
    let open = Process::new("bash")
        .args(["-c", &other.opens()])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    let open = String::from_utf8_lossy(&open.stdout);
    assert!(
        open.contains("read-opened") && open.contains("write-opened"),
        "{open}"
    );
}
