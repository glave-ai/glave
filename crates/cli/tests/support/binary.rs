//! Shared helpers for the binary's end-to-end tests: a throwaway project and home, and `glave`
//! run against a fake model server with nothing from the person's own setup.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;

use glave_ports::testing::{FAKE_SECRET, Running, TempDir};
use glave_provider_openai::fake::FakeServer;
use serde_json::Value;

/// A project folder and a glave home for one test, inside the system temp folder.
pub(crate) struct Place {
    pub project: PathBuf,
    pub home: PathBuf,
    /// Handed to every glave started for the place, so each ends with the test.
    pub lifeline: Lifeline,
    /// The folder both are in, removed when the place drops; none for a place made of
    /// another's folders.
    pub folder: Option<TempDir>,
}

pub(crate) fn place(name: &str) -> Place {
    // Named without a prefix: a longer path would be too long for its server's socket.
    let dir = TempDir::new(name).unwrap();
    let project = dir.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("README.md"), "# demo\n").unwrap();
    Place {
        project,
        home: dir.join("home"),
        lifeline: Lifeline::new(&dir),
        folder: Some(dir),
    }
}

/// The folder goes with the place, and the test fails, unless it fails already, when it could
/// not be removed, most likely as something the test started still wrote there. So a test
/// stops and waits for every program it starts for a place before the place goes: declared
/// after it, or before it in a struct. The lifeline goes after the folder: it still ends a
/// glave a test left running, but too late to keep it from writing there.
impl Drop for Place {
    fn drop(&mut self) {
        if let Some(folder) = self.folder.take()
            && let Err(error) = folder.remove()
            && !std::thread::panicking()
        {
            panic!("{error}; something the test started may still write there");
        }
    }
}

/// A socket a test listens on and hands every glave it starts (`GLAVE_LIFELINE`), so they end
/// when the test does, however it ends, killed outright included, and so does the person's
/// server one of them starts in the background (`specs/one-server.md`, "No orphaned
/// processes"). Each glave's connection waits unanswered; the system closes them all when the
/// listener goes. Clones share it; it goes when the last does.
#[derive(Clone)]
pub(crate) struct Lifeline {
    pub path: PathBuf,
    _held: Arc<Listening>,
}

/// The listener, and its socket's file, removed when it goes.
struct Listening {
    _listener: std::os::unix::net::UnixListener,
    path: PathBuf,
}

impl Drop for Listening {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Lifeline {
    /// The variable that hands it to glave.
    pub(crate) const VARIABLE: &str = "GLAVE_LIFELINE";

    /// A new one for `folder`, listening.
    pub(crate) fn new(folder: &Path) -> Self {
        let path = socket_path(folder, "lifeline");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        Self {
            path: path.clone(),
            _held: Arc::new(Listening {
                _listener: listener,
                path,
            }),
        }
    }
}

/// Where a socket named `name` for `folder` goes: in it, or, where that path would be too long
/// for a socket's (a deep system temp folder), in `/tmp`, named after the temp folder `folder`
/// is in (`glave-<name>-<pid>-<n>`, this process's own), and any file a killed run left there
/// removed first.
pub(crate) fn socket_path(folder: &Path, name: &str) -> PathBuf {
    let path = folder.join(name);
    let Some(owner) = folder.parent().and_then(Path::file_name) else {
        return path;
    };
    if path.as_os_str().len() <= 100 {
        return path;
    }
    let mut short = owner.to_os_string();
    short.push(format!(".{name}"));
    let short = Path::new("/tmp").join(short);
    let _ = std::fs::remove_file(&short);
    short
}

/// Whether the process `pid` still runs (or waits to be reaped).
pub(crate) fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}

/// Waits up to `limit` for `done`, looking every 50 ms.
pub(crate) fn until(limit: std::time::Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = std::time::Instant::now() + limit;
    while !done() {
        if std::time::Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    true
}

/// Sets `key = value` in `place`'s own settings (`config.toml` in its home), as the person
/// chooses how glave runs: a turn takes no flag for it.
pub(crate) fn choose(place: &Place, key: &str, value: &str) {
    std::fs::create_dir_all(&place.home).unwrap();
    let config = place.home.join("config.toml");
    let own = std::fs::read_to_string(&config).unwrap_or_default();
    let mut lines = vec![format!("{key} = {value}")];
    lines.extend(
        own.lines()
            .filter(|line| !line.starts_with(&format!("{key} =")))
            .map(str::to_owned),
    );
    std::fs::write(&config, lines.join("\n") + "\n").unwrap();
}

/// `glave` with `args`, talking only to `server`, with nothing from the person's own setup.
pub(crate) fn glave(place: &Place, server: &FakeServer, args: &[&str], stdin: &str) -> Output {
    let mut child = start(place, server, args);
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

/// `glave` started with `args`, still running, and stopped when dropped.
pub(crate) fn start(place: &Place, server: &FakeServer, args: &[&str]) -> Running {
    start_with(place, server, args, &[])
}

/// `glave` started with `args` and the variables `env`, still running, and stopped when
/// dropped.
pub(crate) fn start_with(
    place: &Place,
    server: &FakeServer,
    args: &[&str],
    env: &[(&str, &str)],
) -> Running {
    let child = Command::new(env!("CARGO_BIN_EXE_glave"))
        .args(args)
        .env(Lifeline::VARIABLE, &place.lifeline.path)
        .envs(env.iter().copied())
        .current_dir(&place.project)
        .env("GLAVE_BASE_URL", server.url())
        .env("GLAVE_HOME", &place.home)
        .env("GLAVE_API_KEY", FAKE_SECRET)
        .env("NO_COLOR", "1")
        .env_remove("GLAVE_MODEL")
        .env_remove("GLAVE_PROVIDER")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    Running::new(child)
}

pub(crate) fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// The text of every message of `role` in a request the model received.
pub(crate) fn said(body: &Value, role: &str) -> Vec<String> {
    body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == role)
        .map(|message| message["content"].to_string())
        .collect()
}

/// `glave` with `args` in a terminal of its own, talking only to `server`, still running and
/// stopped when dropped: what is written to its input is typed at that terminal (Ctrl-C is
/// `\x03`), and the terminal closes when it ends. No screen, so no browser opens.
pub(crate) fn terminal(place: &Place, server: &FakeServer, args: &[&str]) -> Running {
    terminal_with(place, server, args, &[])
}

/// As [`terminal`], with the variables `env` too.
pub(crate) fn terminal_with(
    place: &Place,
    server: &FakeServer,
    args: &[&str],
    env: &[(&str, &str)],
) -> Running {
    let spawn = "import pty, sys; sys.exit(pty.spawn(sys.argv[1:]) >> 8)";
    let python = [&["-c", spawn, env!("CARGO_BIN_EXE_glave")][..], args].concat();
    std::fs::create_dir_all(&place.home).unwrap();
    let child = Command::new("python3")
        .args(python)
        .current_dir(&place.project)
        .env("GLAVE_BASE_URL", server.url())
        .env("GLAVE_HOME", &place.home)
        .env("GLAVE_API_KEY", FAKE_SECRET)
        .env("NO_COLOR", "1")
        .env("SSH_CONNECTION", "test")
        .env(Lifeline::VARIABLE, &place.lifeline.path)
        .env_remove("GLAVE_MODEL")
        .env_remove("GLAVE_PROVIDER")
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    Running::new(child)
}

/// Types `keys` at the terminal `running` is in.
pub(crate) fn typed(running: &mut Running, keys: &str) {
    let input = running.stdin.as_mut().unwrap();
    input.write_all(keys.as_bytes()).unwrap();
    input.flush().unwrap();
}

/// The person's server in `place`, stopped when this drops, however the test ends: asked to
/// stop, and if that fails, whatever still holds its lock killed. Never panics, so a failed
/// test still reports its own failure.
pub(crate) struct Stopped<'a>(pub(crate) &'a Place);

impl Drop for Stopped<'_> {
    fn drop(&mut self) {
        let stopped = Command::new(env!("CARGO_BIN_EXE_glave"))
            .args(["server", "stop"])
            .env("GLAVE_HOME", &self.0.home)
            .env(Lifeline::VARIABLE, &self.0.lifeline.path)
            .stdin(Stdio::null())
            .output();
        if stopped.is_ok_and(|stopped| stopped.status.success()) {
            return;
        }
        let lock = self.0.home.join("run/server.lock");
        let Ok(holders) = Command::new("lsof").arg("-t").arg(&lock).output() else {
            return;
        };
        for pid in String::from_utf8_lossy(&holders.stdout).split_whitespace() {
            let _ = Command::new("kill").args(["-KILL", pid]).status();
        }
    }
}
