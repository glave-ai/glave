//! What a terminal's shell starts on: a pseudo-terminal, the person's own login shell, and the
//! few variables a login shell starts from, its profile setting the rest as in any terminal.

use std::ffi::OsString;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};

use glave_ports::computer::{ComputerError, TerminalSize};

/// The variables a terminal's shell is given from the computer's environment; its login
/// profile sets the rest, as a terminal app's shell starts.
const KEPT: &[&str] = &["HOME", "USER", "LOGNAME", "SHELL", "PATH", "LANG"];

/// The shells an account is given so it opens none.
const NO_SHELL: &[&str] = &["nologin", "false"];

pub(super) fn winsize(size: TerminalSize) -> rustix::termios::Winsize {
    rustix::termios::Winsize {
        ws_row: size.rows,
        ws_col: size.cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

/// A pseudo-terminal: the side glave holds, the side its shell runs in, and on macOS that side's
/// name, by which the processes in it are found there.
pub(super) struct Pty {
    pub(super) master: OwnedFd,
    pub(super) terminal: OwnedFd,
    #[cfg(target_os = "macos")]
    pub(super) name: String,
}

/// A new pseudo-terminal of `size`. Both its sides are closed for every program glave starts as
/// they are made, never a moment after, so no command started meanwhile holds them.
pub(super) fn pseudo_terminal(size: TerminalSize) -> std::io::Result<Pty> {
    use rustix::fs::{Mode, OFlags};
    use rustix::pty::{grantpt, ptsname, unlockpt};
    let master = controlling_side()?;
    grantpt(&master)?;
    unlockpt(&master)?;
    let name = ptsname(&master, Vec::new())?;
    let flags = OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC;
    let terminal = rustix::fs::open(name.as_c_str(), flags, Mode::empty())?;
    rustix::termios::tcsetwinsize(&master, winsize(size))?;
    Ok(Pty {
        master,
        terminal,
        #[cfg(target_os = "macos")]
        name: name.to_string_lossy().into_owned(),
    })
}

#[cfg(target_os = "linux")]
fn controlling_side() -> std::io::Result<OwnedFd> {
    use rustix::pty::{OpenptFlags, openpt};
    Ok(openpt(
        OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC,
    )?)
}

#[cfg(not(target_os = "linux"))]
fn controlling_side() -> std::io::Result<OwnedFd> {
    use rustix::fs::{Mode, OFlags};
    let flags = OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC;
    Ok(rustix::fs::open("/dev/ptmx", flags, Mode::empty())?)
}

/// What a terminal's shell is given of `environment`: the few a login shell starts from, and a
/// terminal of xterm's kind.
pub(super) fn shell_environment(environment: &[(OsString, OsString)]) -> Vec<(OsString, OsString)> {
    environment
        .iter()
        .filter(|(name, _)| KEPT.iter().any(|kept| name == kept))
        .cloned()
        .chain([("TERM".into(), "xterm-256color".into())])
        .collect()
}

/// What a login shell is started with: `-l`, and for bash, which leaves its jobs running as it
/// exits unless told otherwise, `huponexit`, so they get its hang-up, as zsh's do by default.
pub(super) fn login_args(shell: &Path) -> &'static [&'static str] {
    if shell.file_name().is_some_and(|name| name == "bash") {
        &["-l", "-O", "huponexit"]
    } else {
        &["-l"]
    }
}

/// The person's login shell: the one `SHELL` names, else their account's; refused when their
/// account is given one that opens no shell.
pub(super) fn login_shell(environment: &[(OsString, OsString)]) -> Result<PathBuf, ComputerError> {
    let named = |name: &str| {
        environment
            .iter()
            .find(|(each, _)| each == name)
            .map(|(_, value)| value.clone())
    };
    let shell = named("SHELL")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_file())
        .or_else(|| account_shell(environment))
        .unwrap_or_else(|| PathBuf::from("/bin/sh"));
    let opens_none = shell
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| NO_SHELL.contains(&name));
    if opens_none {
        return Err(ComputerError::Refused(
            format!(
                "this account's login shell, {}, opens no shell, so no terminal opens for it",
                shell.display()
            )
            .into(),
        ));
    }
    Ok(shell)
}

/// The account's own shell, from the user database: its line in `/etc/passwd` on Linux.
#[cfg(target_os = "linux")]
fn account_shell(_environment: &[(OsString, OsString)]) -> Option<PathBuf> {
    let uid = rustix::process::getuid().as_raw().to_string();
    let accounts = std::fs::read_to_string("/etc/passwd").ok()?;
    passwd_shell(&accounts, &uid)
}

/// The shell of the account with `uid` in a `/etc/passwd`'s `text`.
#[cfg(any(target_os = "linux", test))]
fn passwd_shell(text: &str, uid: &str) -> Option<PathBuf> {
    text.lines()
        .map(|line| line.split(':').collect::<Vec<_>>())
        .find(|fields| fields.get(2) == Some(&uid))
        .and_then(|fields| fields.get(6).map(|shell| PathBuf::from(*shell)))
        .filter(|shell| shell.is_absolute())
}

/// How long the user database has to answer, which it does in moments.
#[cfg(target_os = "macos")]
const DSCL_ANSWERS_WITHIN: std::time::Duration = std::time::Duration::from_secs(5);

/// The account's own shell, from the user database: Directory Services on macOS.
#[cfg(target_os = "macos")]
fn account_shell(environment: &[(OsString, OsString)]) -> Option<PathBuf> {
    let user = environment
        .iter()
        .find(|(name, _)| name == "USER" || name == "LOGNAME")
        .map(|(_, user)| user.clone())?;
    let mut dscl = glave_process::Program::new("/usr/bin/dscl");
    dscl.arg(".")
        .arg("-read")
        .arg(Path::new("/Users").join(user))
        .arg("UserShell")
        .no_input();
    let read = dscl.output_blocking(DSCL_ANSWERS_WITHIN).ok()?;
    let text = String::from_utf8_lossy(&read.stdout).into_owned();
    let shell = text.strip_prefix("UserShell:")?.trim();
    Some(PathBuf::from(shell)).filter(|shell| shell.is_absolute())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn account_shell(_environment: &[(OsString, OsString)]) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shell_is_the_one_named_else_the_account_s_and_never_one_that_opens_none() {
        let passwd = "root:x:0:0::/root:/bin/sh\nada:x:501:20::/home/ada:/bin/zsh\n";
        assert_eq!(passwd_shell(passwd, "501"), Some(PathBuf::from("/bin/zsh")));
        assert_eq!(passwd_shell(passwd, "7"), None);
        let with = |shell: &str| vec![(OsString::from("SHELL"), OsString::from(shell))];
        assert_eq!(
            login_shell(&with("/bin/sh")).unwrap(),
            PathBuf::from("/bin/sh")
        );
        let refused = login_shell(&with("/usr/bin/false"));
        assert!(
            matches!(refused, Err(ComputerError::Refused(_))),
            "{refused:?}"
        );
        let kept = shell_environment(&[
            ("PATH".into(), "/bin".into()),
            ("GLAVE_HOME".into(), "/g".into()),
        ]);
        let names: Vec<_> = kept.iter().map(|(name, _)| name.clone()).collect();
        assert_eq!(names, [OsString::from("PATH"), OsString::from("TERM")]);
    }
}
