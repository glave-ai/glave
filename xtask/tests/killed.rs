//! What xtask runs ends when xtask is killed outright (#558): `cargo xtask eval`, given a
//! stand-in `cargo` that only sleeps, has it stopped at once by xtask's lifeline.

use std::os::unix::fs::PermissionsExt as _;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use glave_ports::testing::{HUNG, Running, TempDir};

/// Waits up to `limit` for `done`.
fn until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    while !done() {
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    true
}

fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}

/// The stand-in, killed when the test ends however it ends, unless the test saw it end: one
/// the test failed to see stopped would sleep on for five minutes.
struct Stopped(Option<u32>);

impl Drop for Stopped {
    fn drop(&mut self) {
        if let Some(pid) = self.0
            && alive(pid)
        {
            let _ = Command::new("kill")
                .args(["-KILL", &pid.to_string()])
                .status();
        }
    }
}

#[test]
fn what_xtask_runs_ends_when_xtask_is_killed_outright() {
    let folder = TempDir::new("xtask-killed").unwrap();
    let started = folder.join("started");
    // Says it started, by its process's id, then sleeps as a long build would.
    let cargo = folder.join("cargo");
    let script = "#!/bin/sh\n\
        echo $$ > \"$XTASK_TEST_STARTED.part\"\n\
        mv \"$XTASK_TEST_STARTED.part\" \"$XTASK_TEST_STARTED\"\n\
        exec sleep 300\n";
    std::fs::write(&cargo, script).unwrap();
    std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:{}", folder.display(), std::env::var("PATH").unwrap());
    let child = Command::new(env!("CARGO_BIN_EXE_xtask"))
        // No `.env` read: the endpoint the evals would use is not this test's.
        .args(["eval", "--env-file", "/dev/null"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("PATH", path)
        .env("XTASK_TEST_STARTED", &started)
        // Its lock file goes with the test's folder.
        .env("TMPDIR", folder.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut xtask = Running::new(child);
    assert!(
        until(HUNG, || started.exists()),
        "the stand-in never started"
    );
    let pid: u32 = std::fs::read_to_string(&started)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let mut stopped = Stopped(Some(pid));

    xtask.kill().unwrap();
    xtask.wait().unwrap();
    assert!(
        until(HUNG, || !alive(pid)),
        "what xtask ran outlives it: {pid}"
    );
    // Gone: its number may be another process's from now on.
    stopped.0 = None;
}
