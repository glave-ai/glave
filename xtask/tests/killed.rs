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

#[test]
fn what_xtask_runs_ends_when_xtask_is_killed_outright() {
    let folder = TempDir::new("xtask-killed").unwrap();
    let started = folder.join("started");
    // Says it started, by its process's id, then sleeps as a long build would.
    let cargo = folder.join("cargo");
    std::fs::write(
        &cargo,
        "#!/bin/sh\necho $$ > \"$1.part\"\nmv \"$1.part\" \"$1\"\nexec sleep 300\n"
            .replace("$1", "$XTASK_TEST_STARTED"),
    )
    .unwrap();
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

    xtask.kill().unwrap();
    xtask.wait().unwrap();
    assert!(
        until(HUNG, || !alive(pid)),
        "what xtask ran outlives it: {pid}"
    );
}
