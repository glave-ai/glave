//! Real commands in the real Linux sandbox. The bubblewrap tests run where bubblewrap starts,
//! and must where `GLAVE_REQUIRE_SANDBOX` is set; the Landlock tests run where it does not
//! but Landlock and a helper (`GLAVE_TEST_SANDBOX_HELPER`, a built glave) are there, and must
//! where `GLAVE_REQUIRE_LANDLOCK` is set. ext4 tells cases apart, so only the exact spellings
//! of protected names are protected, and those are what these check.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command as Process;

use glave_ports::block_on;
use glave_ports::computer::{
    Command, ComputerError, Computers, Ending, Environment, Output, Sandbox, SandboxLevel,
    SandboxNetwork, Sources, WorkspaceId,
};

use super::landlocked;
use crate::local::sandbox::tests::terminals::AnotherTerminal;
use crate::local::sandbox::tests::token;
use crate::local::sandbox_available;
use crate::{CancelSwitch, LocalComputers};

mod made_again;

fn bubblewrap() -> bool {
    let available = sandbox_available();
    assert!(
        available || std::env::var_os("GLAVE_REQUIRE_SANDBOX").is_none(),
        "GLAVE_REQUIRE_SANDBOX is set but bubblewrap cannot start here"
    );
    available
}

/// The helper to run the Landlock tests with, where they run.
fn landlock() -> Option<PathBuf> {
    let helper = std::env::var_os("GLAVE_TEST_SANDBOX_HELPER").map(PathBuf::from);
    let runs = !sandbox_available() && landlocked::available() && helper.is_some();
    assert!(
        runs || std::env::var_os("GLAVE_REQUIRE_LANDLOCK").is_none(),
        "GLAVE_REQUIRE_LANDLOCK is set but bubblewrap starts, Landlock is missing, or \
         GLAVE_TEST_SANDBOX_HELPER is unset"
    );
    helper.filter(|_| runs)
}

/// A folder outside the system temp folder, which the sandbox lets every command write.
fn scratch(name: &str) -> glave_ports::testing::TempDir {
    let dir = glave_ports::testing::TempDir::outside_temp(&format!("linux-{name}")).unwrap();
    std::fs::create_dir_all(dir.join("project")).unwrap();
    dir
}

#[derive(Default)]
struct Collect(std::sync::Mutex<String>);

impl Output for Collect {
    fn write<'a>(&'a self, text: &'a str) -> glave_ports::BoxFuture<'a, ()> {
        if let Ok(mut all) = self.0.lock() {
            all.push_str(text);
        }
        Box::pin(std::future::ready(()))
    }
}

/// Runs `script` in `dir/project`, in a sandbox with `dir` as the home folder, or none.
fn run(
    dir: &Path,
    sandbox: Option<(SandboxLevel, bool)>,
    helper: Option<&Path>,
    script: &str,
) -> Result<(Ending, String), ComputerError> {
    let environment: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(name, _)| name == "PATH")
        .chain([("HOME".into(), dir.as_os_str().to_owned())])
        .collect();
    let mut computers = LocalComputers::new(dir.join("copies"), environment);
    if let Some(helper) = helper {
        computers = computers.with_helper(helper);
    }
    let mut command = Command::new(script);
    command.sandbox = sandbox.map(|(level, network)| Sandbox {
        level,
        network: SandboxNetwork::when(network),
        home: Some(dir.to_owned()),
        glave_home: Some(dir.join(".glave")),
    });
    block_on!(async {
        let computer = computers.create(&Environment::host()).await?;
        let sources = Sources::Folder {
            path: dir.join("project").to_string_lossy().into_owned(),
        };
        let workspace: WorkspaceId = computers.open(computer.id, &sources).await?.id;
        let output = Collect::default();
        let finished = computers
            .run(workspace, &command, &output, &CancelSwitch::default())
            .await?;
        Ok((finished.ending, output.0.lock().unwrap().clone()))
    })
}

fn git_init(project: &Path) {
    let made = Process::new("git")
        .args(["init", "-q"])
        .current_dir(project)
        .status()
        .unwrap();
    assert!(made.success());
}

#[test]
fn bubblewrap_keeps_protected_paths_read_only_even_moved_or_not_there_yet() {
    if !bubblewrap() {
        return;
    }
    let dir = scratch("protected");
    let project = dir.join("project");
    git_init(&project);
    std::fs::create_dir_all(project.join(".git/worktrees/w")).unwrap();
    std::fs::create_dir_all(project.join(".git/modules/m/hooks")).unwrap();
    std::fs::write(project.join(".git/hooks/existing"), "#!/bin/sh\n").unwrap();
    let config = std::fs::read_to_string(project.join(".git/config")).unwrap();
    let attempts = [
        "echo x > .git/hooks/pre-commit",
        "echo x >> .git/config",
        "echo x > .git/config.worktree",
        "echo x > .git/worktrees/w/config",
        "mkdir -p .git/worktrees/new && echo x > .git/worktrees/new/config",
        "echo x > .git/modules/m/hooks/post-checkout",
        "mkdir -p .glave && echo x > .glave/settings.json",
        "echo x > .mcp.json",
        "mkdir -p .vscode && echo x > .vscode/tasks.json",
        "echo x > .envrc",
        "echo x > HEAD",
        "mv .git/hooks moved-hooks",
        "mv .git moved-git && mkdir -p .git/hooks && echo x > .git/hooks/pre-commit",
        "rm -rf .git/hooks",
    ];
    let script =
        attempts.join(" ; ") + " ; echo x > ordinary.txt && echo x > .git/scratch && echo done";
    let (_, said) = run(&dir, Some((SandboxLevel::Project, false)), None, &script).unwrap();
    assert!(said.contains("done"), "{said}");
    assert!(project.join("ordinary.txt").exists(), "{said}");
    assert!(
        project.join(".git/scratch").exists(),
        "the rest of .git is writable: {said}"
    );
    // Moving the hooks only copies them (a mount point cannot move); the originals stay.
    assert!(project.join(".git/hooks/existing").exists(), "{said}");
    assert!(!project.join(".git/hooks/pre-commit").exists(), "{said}");
    assert_eq!(
        std::fs::read_to_string(project.join(".git/config")).unwrap(),
        config
    );
    for gone in [
        ".git/config.worktree",
        ".git/worktrees/w/config",
        ".git/worktrees/new",
        ".git/modules/m/hooks/post-checkout",
        ".glave",
        ".mcp.json",
        ".vscode",
        ".envrc",
        "HEAD",
        "moved-git",
    ] {
        assert!(!project.join(gone).exists(), "{gone}: {said}");
    }
    // Control: without the sandbox the same hook is written, so the check above can fail.
    run(&dir, None, None, "echo x > .git/hooks/pre-commit").unwrap();
    assert!(project.join(".git/hooks/pre-commit").exists());
}

#[test]
fn bubblewrap_refuses_a_protected_path_that_is_a_link() {
    if !bubblewrap() {
        return;
    }
    let dir = scratch("link");
    std::fs::create_dir_all(dir.join("elsewhere")).unwrap();
    std::os::unix::fs::symlink(dir.join("elsewhere"), dir.join("project/.vscode")).unwrap();
    let refused = run(&dir, Some((SandboxLevel::Project, false)), None, "true");
    assert!(
        matches!(&refused, Err(ComputerError::Refused(why)) if why.contains(".vscode")),
        "{refused:?}"
    );
}

/// A process pool whose children Python's forkserver starts, which listens on a Unix socket in
/// the command's temp folder.
const POOL: &str = "import multiprocessing as m
if __name__ == '__main__':
    m.set_start_method('forkserver')
    with m.Pool(2) as pool:
        print('pooled', sum(pool.map(abs, [-1, -2])))
";

/// Something listening at a Unix socket where a daemon's would be, removed when dropped, with
/// its folder if this made it.
struct Daemon {
    socket: PathBuf,
    made: Option<PathBuf>,
    _listener: std::os::unix::net::UnixListener,
}

impl Daemon {
    /// Where tmux's servers listen.
    fn tmux() -> Self {
        Self::listen(format!("/tmp/tmux-{}", rustix::process::getuid().as_raw()))
    }

    /// Where sshd keeps an agent it forwards to this machine: `ssh-` and ten letters and
    /// digits, open to its owner alone.
    fn forwarded_agent() -> Self {
        Self::listen(format!("/tmp/ssh-glave{:05}", std::process::id() % 100_000))
    }

    /// At a socket of this test's own in `folder`, made for its owner alone if it is not there.
    fn listen(folder: String) -> Self {
        use std::os::unix::fs::DirBuilderExt as _;
        let folder = PathBuf::from(folder);
        let made = match std::fs::DirBuilder::new().mode(0o700).create(&folder) {
            Ok(()) => Some(folder.clone()),
            // There already: the person's own, as their tmux's is where they use it.
            Err(error) => {
                assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists, "{error}");
                None
            }
        };
        let socket = folder.join(format!("glave-test-{}", std::process::id()));
        Self::bound(socket, made)
    }

    /// At `socket` itself, a file in a folder that is there.
    fn at(socket: String) -> Self {
        Self::bound(PathBuf::from(socket), None)
    }

    fn bound(socket: PathBuf, made: Option<PathBuf>) -> Self {
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        Self {
            socket,
            made,
            _listener: listener,
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket);
        if let Some(folder) = &self.made {
            let _ = std::fs::remove_dir(folder);
        }
    }
}

/// A command connecting to the Unix socket at `socket`, which prints `connected` if it can.
fn connect(socket: &Path) -> String {
    format!(
        "python3 -c \"import socket; s = socket.socket(socket.AF_UNIX); s.connect('{}'); print('connected')\"",
        socket.display()
    )
}

/// Without the network no Unix socket on the machine is reached. With it, a command makes its
/// own (a process pool works) and reaches one elsewhere, as Codex lets it, but not where the
/// person's daemons listen, which is hidden. `ptrace` is refused either way.
#[test]
fn bubblewrap_lets_unix_sockets_with_the_network_but_hides_the_daemons_and_blocks_ptrace() {
    if !bubblewrap() {
        return;
    }
    let dir = scratch("seccomp");
    let socket = glave_ports::testing::SocketPath::new(&dir, "outside.sock").unwrap();
    let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    let tmux = Daemon::tmux();
    std::fs::write(dir.join("project/pool.py"), POOL).unwrap();
    let sandboxed = |network: bool, script: &str| {
        run(&dir, Some((SandboxLevel::Project, network)), None, script)
            .unwrap()
            .1
    };
    let (_, open) = run(&dir, None, None, &connect(&socket)).unwrap();
    assert!(open.contains("connected"), "{open}");
    let (_, open) = run(&dir, None, None, &connect(&tmux.socket)).unwrap();
    assert!(open.contains("connected"), "control, tmux's: {open}");

    let offline = sandboxed(false, &connect(&socket));
    assert!(!offline.contains("connected"), "{offline}");
    let online = sandboxed(true, &connect(&socket));
    assert!(online.contains("connected"), "{online}");
    let daemon = sandboxed(true, &connect(&tmux.socket));
    assert!(!daemon.contains("connected"), "tmux's: {daemon}");
    let pool = sandboxed(true, "python3 pool.py");
    assert_eq!(pool.trim(), "pooled 3");

    let ptrace =
        "python3 -c \"import ctypes; print('ptrace', ctypes.CDLL(None).ptrace(0, 0, 0, 0))\"";
    let said = sandboxed(true, ptrace);
    assert!(said.contains("ptrace -1"), "{said}");
}

/// Whether a command connects to `daemon`'s socket: in the project's sandbox with the network
/// on, or outside any.
fn reached(dir: &Path, sandboxed: bool, daemon: &Daemon) -> bool {
    let sandbox = sandboxed.then_some((SandboxLevel::Project, true));
    let (_, said) = run(dir, sandbox, None, &connect(&daemon.socket)).unwrap();
    said.contains("connected")
}

/// What listens in `/tmp` for the person's other programs has no fixed name, so it is looked
/// for before each command: an agent sshd forwarded (no `SSH_AUTH_SOCK` names it), a connection
/// ssh shares, Neovim's and VS Code's servers. With the network on a command reaches none of
/// them, and still reaches a folder that only looks like sshd's, which is someone's own.
#[test]
fn bubblewrap_hides_what_listens_in_tmp_for_the_person_s_other_programs() {
    if !bubblewrap() {
        return;
    }
    let dir = scratch("listening");
    let pid = std::process::id();
    let theirs = [
        Daemon::forwarded_agent(),
        Daemon::listen(format!("/tmp/nvim.glave{pid}")),
        Daemon::at(format!("/tmp/ssh-glave@test:{pid}")),
        Daemon::at(format!("/tmp/ssh_mux_test_{pid}_glave")),
        Daemon::at(format!("/tmp/vscode-ipc-glave{pid}.sock")),
        Daemon::at(format!("/tmp/vscode-git-glave{pid}.sock")),
    ];
    let unrelated = Daemon::listen(format!("/tmp/ssh-glave-notes-{pid}"));
    for daemon in theirs.iter().chain([&unrelated]) {
        let socket = daemon.socket.display();
        assert!(reached(&dir, false, daemon), "control, {socket}");
    }
    for daemon in &theirs {
        assert!(!reached(&dir, true, daemon), "{}", daemon.socket.display());
    }
    assert!(reached(&dir, true, &unrelated), "someone's own folder");
}

/// Every place a daemon or a session listens is hidden, proved where it can be: the ones kept
/// in `/tmp`, made here with something listening (GNU screen's, Zellij's, Emacs's), and
/// whichever of the rest this machine has, which only root could make: on CI's runners,
/// Docker's socket and the system bus. tmux's is the test's above, which makes it while this
/// one runs.
#[test]
fn bubblewrap_hides_every_place_a_daemon_or_a_session_listens() {
    if !bubblewrap() {
        return;
    }
    let dir = scratch("daemons");
    let uid = rustix::process::getuid().as_raw();
    let made = [
        "/tmp/uscreens".to_owned(),
        format!("/tmp/zellij-{uid}"),
        format!("/tmp/emacs{uid}"),
    ]
    .map(Daemon::listen);
    for daemon in &made {
        let socket = daemon.socket.display();
        assert!(reached(&dir, false, daemon), "control, {socket}");
        assert!(!reached(&dir, true, daemon), "{socket}");
    }
    // Each place there is, a socket or a folder with something in it, shows as neither.
    let tmux = PathBuf::from(format!("/tmp/tmux-{uid}"));
    let look: Vec<String> = super::mounts::daemons(None, uid, None)
        .iter()
        .filter(|place| **place != tmux)
        .map(|place| {
            let place = place.display();
            format!(
                "if [ -S '{place}' ] || {{ [ -d '{place}' ] && \
                 [ -n \"$(ls -A '{place}' 2>/dev/null)\" ]; }}; then echo 'reached {place}'; fi"
            )
        })
        .chain(["true".to_owned()])
        .collect();
    let look = look.join("\n");
    let (_, open) = run(&dir, None, None, &look).unwrap();
    assert!(open.contains("reached /tmp/uscreens"), "control: {open}");
    let (_, said) = run(&dir, Some((SandboxLevel::Project, true)), None, &look).unwrap();
    assert!(!said.contains("reached"), "{said}");
}

#[test]
fn landlock_confines_read_only_commands() {
    let Some(helper) = landlock() else {
        return;
    };
    let dir = scratch("landlock");
    let project = dir.join("project");
    std::fs::create_dir_all(dir.join(".ssh")).unwrap();
    std::fs::write(dir.join(".ssh/id_ed25519"), "canary-ssh\n").unwrap();
    std::fs::create_dir_all(dir.join(".glave")).unwrap();
    std::fs::write(dir.join(".glave/glave.db"), "canary-db\n").unwrap();
    std::fs::write(project.join(".env"), "env-ok\n").unwrap();
    std::fs::write(project.join("notes.md"), "notes-ok\n").unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let script = format!(
        "cat {home}/.ssh/id_ed25519 {home}/.glave/glave.db .env notes.md; \
         echo x > inside.txt; echo x > {home}/outside.txt; \
         echo t > \"$TMPDIR/t\" && echo temp-ok; echo t > /tmp/glave-landlock-$$ && rm /tmp/glave-landlock-$$ && echo tmp-ok; \
         (exec 3<>/dev/tcp/127.0.0.1/{port}) && echo connected; \
         cat /proc/{me}/environ >/dev/null && echo environ-read; true",
        home = dir.display(),
        me = std::process::id(),
    );
    let (_, said) = run(
        &dir,
        Some((SandboxLevel::ReadOnly, false)),
        Some(&helper),
        &script,
    )
    .unwrap();
    assert!(!said.contains("canary"), "{said}");
    // The project's own `.env` is the project's: commands read it.
    assert!(
        said.contains("notes-ok") && said.contains("env-ok"),
        "{said}"
    );
    assert!(
        said.contains("temp-ok") && said.contains("tmp-ok"),
        "{said}"
    );
    assert!(!said.contains("connected"), "{said}");
    // Landlock leaves `/proc` the machine's, but not another process's environment.
    assert!(!said.contains("environ-read"), "{said}");
    assert!(!project.join("inside.txt").exists(), "{said}");
    assert!(!dir.join("outside.txt").exists(), "{said}");
    // Control: unconfined, every canary is readable and the write lands.
    let (_, open) = run(&dir, None, None, &script).unwrap();
    assert_eq!(open.matches("canary").count(), 2, "{open}");
    assert!(open.contains("environ-read"), "{open}");
    assert!(dir.join("outside.txt").exists());
}

/// Under Landlock a command opens no other session's terminal either, though the same script
/// outside the sandbox opens it both ways. (bubblewrap's `/dev` is its own:
/// `sandbox/tests/terminals.rs`.)
#[test]
fn landlock_hides_other_terminals() {
    let Some(helper) = landlock() else {
        return;
    };
    let dir = scratch("landlock-terminals");
    let other = AnotherTerminal::open();
    let script = format!("{}; tty; echo done", other.opens());
    let (_, said) = run(
        &dir,
        Some((SandboxLevel::ReadOnly, false)),
        Some(&helper),
        &script,
    )
    .unwrap();
    assert!(
        said.contains("not a tty") && said.contains("done"),
        "{said}"
    );
    assert!(!said.contains("opened"), "{said}");
    let (_, open) = run(&dir, None, None, &script).unwrap();
    assert!(
        open.contains("read-opened") && open.contains("write-opened"),
        "{open}"
    );
}

/// Landlock's plan names every terminal's device but `tty`, the command's own.
#[test]
fn every_terminal_but_the_command_s_own_is_named() {
    let dev = glave_ports::testing::TempDir::new("dev").unwrap();
    for name in ["tty", "tty1", "ttyS0", "ptyp0", "ptmx", "null"] {
        std::fs::write(dev.join(name), "").unwrap();
    }
    std::fs::create_dir(dev.join("pts")).unwrap();
    let mut named = super::other_terminals(&dev);
    named.sort();
    let mut expected: Vec<PathBuf> = ["pts", "tty1", "ttyS0", "ptyp0"]
        .iter()
        .map(|name| dev.join(name))
        .collect();
    expected.sort();
    assert_eq!(named, expected);
}

/// Commands in bubblewrap have processes of their own: none of glave's, or anything else on
/// the machine, shows in their `/proc`, so no other program's arguments or environment can be
/// read there.
#[test]
fn bubblewrap_shows_no_process_but_the_command_s_own() {
    if !bubblewrap() {
        return;
    }
    let dir = scratch("processes");
    let me = std::process::id();
    let this = std::env::current_exe().unwrap();
    let name = this.file_name().unwrap().to_string_lossy().into_owned();
    let script = format!("tr '\\0' ' ' < /proc/{me}/cmdline; true");
    let (_, said) = run(&dir, Some((SandboxLevel::Project, true)), None, &script).unwrap();
    assert!(!said.contains(&name), "{said}");
    // Control: unconfined, this test's own command line is there.
    let (_, open) = run(&dir, None, None, &script).unwrap();
    assert!(open.contains(&name), "{open}");
}

/// Computers with `helper`, the read-only sandbox Landlock runs, with `dir` as the home folder
/// and `dir/.glave` as glave's folder, and `dir/project` opened on one of them, its root.
fn landlocked(dir: &Path, helper: &Path) -> (LocalComputers, Sandbox, WorkspaceId, PathBuf) {
    let glave_home = dir.join(".glave");
    std::fs::create_dir_all(&glave_home).unwrap();
    let environment: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(name, _)| name == "PATH")
        .chain([("HOME".into(), dir.as_os_str().to_owned())])
        .collect();
    let computers = LocalComputers::new(dir.join("copies"), environment).with_helper(helper);
    // As `glave computer serve` makes them, the folder of a computer's token hidden too.
    let computers = token::serving(computers, &glave_home);
    let sandbox = Sandbox {
        level: SandboxLevel::ReadOnly,
        network: SandboxNetwork::Off,
        home: Some(dir.to_path_buf()),
        glave_home: Some(glave_home),
    };
    let root = dir.join("project");
    let workspace = block_on!(async {
        let computer = computers.create(&Environment::host()).await.unwrap();
        let sources = Sources::Folder {
            path: root.to_string_lossy().into_owned(),
        };
        computers.open(computer.id, &sources).await.unwrap().id
    });
    (computers, sandbox, workspace, root)
}

/// Under Landlock too, what glave writes in its folder while a command runs stays hidden from
/// it, and its package cache works.
#[test]
fn landlock_keeps_what_glave_writes_later_hidden() {
    let Some(helper) = landlock() else {
        return;
    };
    let dir = scratch("landlock-late");
    let (computers, sandbox, here, root) = landlocked(&dir, &helper);
    block_on!(super::super::tests::written_later_stays_hidden(
        &computers, here, &root, &sandbox
    ));
}

/// Under Landlock too, a computer's token in glave's folder is neither read nor replaced, nor
/// its key read (#1502).
#[test]
fn landlock_keeps_a_computer_s_token_unread_and_unreplaced() {
    let Some(helper) = landlock() else {
        return;
    };
    let dir = scratch("landlock-token");
    let (computers, sandbox, here, _) = landlocked(&dir, &helper);
    block_on!(token::kept_from_a_command(&computers, here, &sandbox));
}

/// Under Landlock too, glave's folder on commands' `PATH` stays readable though the rest of
/// glave's folder is hidden, so a command's `glave` is the one that runs it.
#[test]
fn landlock_keeps_glave_on_path_readable() {
    let Some(helper) = landlock() else {
        return;
    };
    let dir = scratch("landlock-on-path");
    let glave_home = dir.join(".glave");
    let on_path = glave_home.join("bin");
    let running = dir.join("running/glave");
    super::super::tests::fake_glave(&running, "running-glave");
    std::fs::create_dir_all(&glave_home).unwrap();
    std::fs::write(glave_home.join("glave.db"), "canary-db\n").unwrap();
    let environment: Vec<(OsString, OsString)> = vec![
        ("PATH".into(), "/usr/bin:/bin".into()),
        ("HOME".into(), dir.as_os_str().to_owned()),
    ];
    let computers = LocalComputers::new(dir.join("copies"), environment)
        .with_helper(&helper)
        .with_glave_on_path(&on_path);
    let sandbox = Sandbox {
        level: SandboxLevel::ReadOnly,
        network: SandboxNetwork::Off,
        home: Some(dir.to_path_buf()),
        glave_home: Some(glave_home.clone()),
    };
    let root = dir.join("project");
    block_on!(async {
        crate::glave_on_path(&on_path, &running).await.unwrap();
        let computer = computers.create(&Environment::host()).await.unwrap();
        let sources = Sources::Folder {
            path: root.to_string_lossy().into_owned(),
        };
        let here = computers.open(computer.id, &sources).await.unwrap().id;
        let mut command = Command::new(format!(
            "glave --version; cat '{}/glave.db'; true",
            glave_home.display()
        ));
        command.sandbox = Some(sandbox);
        let output = Collect::default();
        computers
            .run(here, &command, &output, &CancelSwitch::default())
            .await
            .unwrap();
        let said = output.0.lock().unwrap().clone();
        assert!(said.contains("running-glave --version"), "{said}");
        assert!(!said.contains("canary"), "{said}");
    });
}

#[test]
fn landlock_runs_with_named_pipes_beside_what_it_hides() {
    let Some(helper) = landlock() else {
        return;
    };
    let dir = scratch("landlock-pipes");
    let project = dir.join("project");
    // Each folder holding something hidden has its entries granted one by one, a pipe too.
    std::fs::create_dir_all(dir.join(".ssh")).unwrap();
    std::fs::write(dir.join(".ssh/id_ed25519"), "canary-ssh\n").unwrap();
    std::fs::write(project.join("notes.md"), "notes-ok\n").unwrap();
    for pipe in [dir.join("pipe"), project.join("pipe")] {
        let made = Process::new("mkfifo").arg(&pipe).status().unwrap();
        assert!(made.success());
    }
    let (ending, said) = run(
        &dir,
        Some((SandboxLevel::ReadOnly, false)),
        Some(&helper),
        &format!("cat notes.md {}/.ssh/id_ed25519", dir.display()),
    )
    .unwrap();
    assert!(
        said.contains("notes-ok") && !said.contains("canary"),
        "{said}"
    );
    assert!(!matches!(ending, Ending::TimedOut { .. }), "{said}");
}

#[test]
fn landlock_refuses_the_project_level_and_says_what_to_install() {
    let Some(helper) = landlock() else {
        return;
    };
    let dir = scratch("landlock-project");
    let refused = run(
        &dir,
        Some((SandboxLevel::Project, false)),
        Some(&helper),
        "true",
    );
    assert!(
        matches!(&refused, Err(ComputerError::Unsupported(why)) if why.contains("install bubblewrap")),
        "{refused:?}"
    );
}
