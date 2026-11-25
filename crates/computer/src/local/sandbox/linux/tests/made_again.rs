//! A private folder in `/tmp` the sandbox hides that is gone by the time bubblewrap mounts
//! over it, which bubblewrap makes again on the host, open to all (#1539).

use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::process::{Command as Process, Stdio};
use std::time::Duration;

use glave_ports::block_on;
use glave_ports::computer::{RunsFor, Sandbox, SandboxLevel, SandboxNetwork};
use glave_process::{Child, Processes, Program};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

use super::{bubblewrap, scratch};
use crate::local::sandbox::{Around, Launch, Prepared};

/// A folder in `/tmp` of the test's own, removed with what is in it when dropped.
struct InTmp(PathBuf);

impl Drop for InTmp {
    fn drop(&mut self) {
        let _ = glave_ports::remove_tree(&self.0);
    }
}

/// `script` run by bash in the sandbox `launch` makes, its output on a pipe.
async fn start(processes: &Processes, launch: &Launch, script: &str) -> Child {
    let mut program = Program::new(&launch.program);
    program
        .args(&launch.args)
        .args(["bash", "-c", script])
        .piped_input()
        .stdout(Stdio::piped());
    let mut child = processes.spawn(&mut program).await.unwrap();
    let mut input = child.stdin.take().unwrap();
    let filter = launch.input.as_deref().unwrap();
    input.write_all(filter).await.unwrap();
    drop(input);
    child
}

/// The folder's mode, `0o777` at most.
fn mode(folder: &Path) -> u32 {
    std::fs::metadata(folder).unwrap().permissions().mode() & 0o777
}

/// Two commands are planned while a private folder in `/tmp` is there, which goes before
/// either's sandbox is made, as tmux's does when its last session ends. The first one's
/// bubblewrap makes it again, open to all, which tmux would refuse; the second mounts its
/// stand-in over that. Once the first ends the folder is private, as tmux makes its own, and
/// still under the second's stand-in: what the person's tmux makes in it next, the second
/// never sees, as it would if the folder had been removed under its mount.
#[test]
fn a_folder_bubblewrap_made_again_is_made_private_and_stays_hidden() {
    if !bubblewrap() {
        return;
    }
    let dir = scratch("made-again");
    let project = std::fs::canonicalize(dir.join("project")).unwrap();
    let place = PathBuf::from(format!("/tmp/glave-test-made-again-{}", std::process::id()));
    let _kept = InTmp(place.clone());
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&place)
        .unwrap();
    let fifo = project.join("go");
    assert!(
        Process::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let sandbox = Sandbox {
        level: SandboxLevel::Project,
        network: SandboxNetwork::when(true),
        home: Some(dir.to_path_buf()),
        glave_home: Some(dir.join(".glave")),
        runs_for: RunsFor::Owner,
    };
    let listing = format!(
        "echo ready; read -r _ < go; ls -A '{}' 2>&1; echo listed",
        place.display()
    );
    let said = block_on!(async {
        let hidden = [place.clone()];
        let around = Around {
            hidden: &hidden,
            glave_on_path: None,
            projects: None,
        };
        let prepared = Prepared::new(&sandbox, &project, None, None, around);
        let prepared = prepared.await.unwrap();
        let first = prepared.launch().await.unwrap();
        let second = prepared.launch().await.unwrap();
        std::fs::remove_dir(&place).unwrap();
        let processes = Processes::unrecorded();

        let mut ran = start(&processes, &first, "true").await;
        let ended = tokio::time::timeout(Duration::from_secs(30), ran.wait()).await;
        assert!(ended.unwrap().unwrap().success());
        assert_eq!(mode(&place), 0o755, "control: made again, open to all");

        let mut running = start(&processes, &second, &listing).await;
        let mut lines = BufReader::new(running.stdout.take().unwrap()).lines();
        let ready = tokio::time::timeout(Duration::from_secs(30), lines.next_line()).await;
        assert_eq!(ready.unwrap().unwrap().as_deref(), Some("ready"));
        drop(first);
        assert_eq!(mode(&place), 0o700, "made private");

        // What the person's tmux makes there next, then the second command looks.
        std::fs::write(place.join("socket"), "").unwrap();
        let go = tokio::task::spawn_blocking(move || std::fs::write(fifo, "go\n"));
        let went = tokio::time::timeout(Duration::from_secs(30), go).await;
        went.unwrap().unwrap().unwrap();
        let mut said = String::new();
        loop {
            let next = tokio::time::timeout(Duration::from_secs(30), lines.next_line()).await;
            let Ok(Ok(Some(line))) = next else {
                break;
            };
            said.push_str(&line);
            said.push('\n');
        }
        let ended = tokio::time::timeout(Duration::from_secs(30), running.wait()).await;
        assert!(ended.unwrap().unwrap().success(), "{said}");
        said
    });
    assert!(said.contains("listed"), "{said}");
    assert!(!said.contains("socket"), "{said}");
}
