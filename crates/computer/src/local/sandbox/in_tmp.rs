//! What listens in `/tmp` for the person's other programs, found afresh before each command on
//! Linux: an ssh agent, a connection ssh shares, an editor's server. Each is named as it
//! starts, so no list holds it ahead, as `linux/mounts.rs` holds the places with fixed names.
//! A command that reached one would sign with the person's keys, open a shell on another
//! machine, or run what it likes in the editor, all outside the sandbox.
//!
//! Only what has one of their shapes and is the person's own is taken, and only so many: a
//! command makes names in `/tmp` too, so a looser match would let it have anything there
//! hidden from the commands after it, and no limit would let it cost each of them thousands of
//! mounts.

use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _};
use std::path::{Path, PathBuf};

/// The most of them one command's mounts hide. Past it the command is refused, never run with
/// the rest left reachable: a limit that dropped some would let a command bury a real one
/// under names of its own.
const MOST: usize = 256;

/// Whether a folder named `name` is where an agent or an editor's server listens: sshd's for
/// an agent it forwards (`ssh-` and ten letters and digits, as `mkdtemp` makes them),
/// ssh-agent's own (twelve), or Neovim's (`nvim.<user>`).
fn folder(name: &str) -> bool {
    let made = |rest: &str| {
        matches!(rest.len(), 10 | 12) && rest.bytes().all(|byte| byte.is_ascii_alphanumeric())
    };
    name.strip_prefix("ssh-").is_some_and(made) || name.starts_with("nvim.")
}

/// Whether a socket named `name` is one of theirs: a connection ssh shares (`ControlPath`, by
/// its usual names, `ssh-%r@%h:%p` and `ssh_mux_%h_%p_%r`), or VS Code's (`vscode-ipc-*.sock`,
/// `vscode-git-*.sock`).
fn socket(name: &str) -> bool {
    let code = ["vscode-ipc-", "vscode-git-"];
    let sock = Path::new(name).extension().is_some_and(|ext| ext == "sock");
    name.starts_with("ssh-")
        || name.starts_with("ssh_mux_")
        || (sock && code.iter().any(|prefix| name.starts_with(prefix)))
}

/// What listens in `tmp` for the other programs of the person `uid` is: each folder of one of
/// `folder`'s shapes that only its owner opens, and each socket of one of `socket`'s, that
/// `uid` owns. Never a link, so a command cannot have another place hidden by making one. An
/// error saying why when there are more than `MOST`. Blocking.
pub(super) fn listeners(tmp: &Path, uid: u32) -> Result<Vec<PathBuf>, String> {
    let Ok(entries) = std::fs::read_dir(tmp) else {
        return Ok(Vec::new());
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // By name first: `/tmp` can hold a great deal, and only these are worth a look.
        if !(folder(&name) || socket(&name)) {
            continue;
        }
        // The entry itself, never what a link leads to.
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let kind = meta.file_type();
        let private_folder = kind.is_dir() && folder(&name) && meta.mode() & 0o7777 == 0o700;
        if meta.uid() == uid && (private_folder || (kind.is_socket() && socket(&name))) {
            found.push(entry.path());
        }
    }
    if found.len() > MOST {
        return Err(format!(
            "the sandbox cannot hide all that listens in {} for your other programs: {} ssh \
             agents, shared ssh connections and editors' servers are there, and it hides {MOST} \
             at most; remove the ones no longer in use, or run without the sandbox",
            tmp.display(),
            found.len()
        ));
    }
    found.sort();
    Ok(found)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::DirBuilderExt as _;
    use std::os::unix::net::UnixListener;

    use glave_ports::testing::{SocketPath, TempDir};

    use super::*;

    /// A folder standing in for `/tmp`, short enough for a socket's path wherever the system's
    /// temp folder is, and the user who owns what is made in it.
    struct Tmp {
        folder: PathBuf,
        uid: u32,
        listeners: Vec<UnixListener>,
        _kept: (SocketPath, TempDir),
    }

    impl Tmp {
        fn new() -> Self {
            let dir = TempDir::new("in-tmp").unwrap();
            let probe = SocketPath::new(&dir, &"s".repeat(40)).unwrap();
            let folder = probe.parent().unwrap().to_owned();
            let uid = std::fs::metadata(&folder).unwrap().uid();
            Self {
                folder,
                uid,
                listeners: Vec::new(),
                _kept: (probe, dir),
            }
        }

        fn folder(&self, name: &str, mode: u32) -> PathBuf {
            let path = self.folder.join(name);
            std::fs::DirBuilder::new().mode(mode).create(&path).unwrap();
            path
        }

        fn socket(&mut self, name: &str) -> PathBuf {
            let path = self.folder.join(name);
            self.listeners.push(UnixListener::bind(&path).unwrap());
            path
        }
    }

    #[test]
    fn agents_shared_connections_and_editors_servers_are_found_by_their_shapes() {
        let mut tmp = Tmp::new();
        let mut theirs = vec![
            tmp.folder("ssh-Ab3dEf9h1J", 0o700),
            tmp.folder("ssh-Ab3dEf9h1Jk2", 0o700),
            tmp.folder("nvim.me", 0o700),
            tmp.socket("ssh-me@host:22"),
            tmp.socket("ssh_mux_host_22_me"),
            tmp.socket("vscode-ipc-0a1b.sock"),
            tmp.socket("vscode-git-2c3d.sock"),
        ];
        theirs.sort();
        // Each only looks like one of them: the wrong length, open to others, not a socket,
        // a link, or another program's own.
        tmp.folder("ssh-foo", 0o700);
        tmp.folder("ssh-Zz3dEf9h1J", 0o755);
        tmp.folder("ssh-notes-2026", 0o700);
        tmp.folder("vscode-ipc-folder.sock", 0o700);
        std::fs::write(tmp.folder.join("ssh-notes"), "a file").unwrap();
        std::fs::write(tmp.folder.join("vscode-ipc-file.sock"), "a file").unwrap();
        std::os::unix::fs::symlink(&tmp.folder, tmp.folder.join("ssh-Li3dEf9h1J")).unwrap();
        tmp.socket("app.sock");
        tmp.socket("vscode-ipc-0a1b");

        assert_eq!(listeners(&tmp.folder, tmp.uid).unwrap(), theirs);
        let someone_else = listeners(&tmp.folder, tmp.uid.wrapping_add(1)).unwrap();
        assert!(someone_else.is_empty(), "{someone_else:?}");
    }

    #[test]
    fn more_than_the_most_is_refused_never_cut_short() {
        let tmp = Tmp::new();
        for n in 0..MOST {
            tmp.folder(&format!("ssh-{n:010}"), 0o700);
        }
        assert_eq!(listeners(&tmp.folder, tmp.uid).unwrap().len(), MOST);
        tmp.folder(&format!("ssh-{MOST:010}"), 0o700);
        let refused = listeners(&tmp.folder, tmp.uid).unwrap_err();
        assert!(
            refused.contains(&format!("{} ssh agents", MOST + 1)),
            "{refused}"
        );
        assert!(refused.contains("run without the sandbox"), "{refused}");
    }

    #[test]
    fn a_folder_that_is_not_there_holds_nothing() {
        let tmp = Tmp::new();
        let found = listeners(&tmp.folder.join("gone"), tmp.uid).unwrap();
        assert!(found.is_empty(), "{found:?}");
    }
}
