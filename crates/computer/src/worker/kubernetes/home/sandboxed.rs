//! glave's own sandbox inside a person's computer (`GLAVE_TEST_MEASURE=1` under gVisor on `kind`,
//! `specs/sandbox.md`, "Under gVisor"): every wall a command meets there, each with its control,
//! as it reaches the computer: across the protocol, its sandbox travelling with the command.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use glave_ports::block_on;
use glave_ports::computer::{
    Command, ComputerError, Computers as _, Ending, Environment, Image, Limits, Network, Sandbox,
    SandboxLevel, Sources, WorkspaceId,
};

use super::super::objects::PERSON_HOME;
use super::super::sizing::Size;
use super::super::tests::{cluster, left_behind};
use super::PersonComputer;
use super::tests::ran;
use crate::worker::backend::Backend as _;
use crate::worker::route::Tail;
use crate::{CancelSwitch, WorkerComputers, WorkerCredentials};

/// A project with a commit, and a secret where glave hides them.
const PREPARE: &str = "mkdir -p ~/.ssh ~/p && echo key > ~/.ssh/id_rsa && cd ~/p && git init -q && \
    git config user.email a@example.com && git config user.name a && echo hello > readme && \
    echo secret > .env && git add readme && git commit -qm one && echo prepared";

/// Tries making a socket of `$1`, and connecting it to `$2` if given: prints what happened.
const SOCKET: &str = r#"
import socket, sys
try:
    s = socket.socket(int(sys.argv[1]), socket.SOCK_STREAM if int(sys.argv[1]) != 16 else socket.SOCK_RAW)
    if len(sys.argv) > 2:
        s.connect(sys.argv[2] if int(sys.argv[1]) == 1 else (sys.argv[2], int(sys.argv[3])))
    print("connected" if len(sys.argv) > 2 else "made")
except OSError as error:
    print("refused", error.errno)
"#;

/// A call of system call `$1`, which prints its errno.
const SYSCALL: &str = r"
import ctypes, sys
libc = ctypes.CDLL(None, use_errno=True)
print(libc.syscall(int(sys.argv[1]), 0, 0, 0, 0), ctypes.get_errno())
";

/// A listener on loopback, and a connection to it from the same command.
const LOOPBACK: &str = r#"
import socket
s = socket.socket(); s.bind(("127.0.0.1", 0)); s.listen(1)
c = socket.socket(); c.connect(s.getsockname()); print("reached")
"#;

/// What a check should come to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Comes {
    /// It exits 0, printing this.
    Prints(&'static str),
    /// It does not exit 0.
    Fails,
}

/// How a command's run ended: its output when it exited 0.
async fn run(
    computers: &WorkerComputers,
    workspace: WorkspaceId,
    sandbox: Option<Sandbox>,
    script: &str,
) -> Result<Result<String, String>, ComputerError> {
    let mut command = Command::new(script);
    command.sandbox = sandbox;
    let output = Tail::default();
    let finished = computers
        .run(workspace, &command, &output, &CancelSwitch::default())
        .await?;
    Ok(match finished.ending {
        Ending::Exited { code: 0 } => Ok(output.text(2_000)),
        ending => Err(format!("{ending:?} {}", output.text(600))),
    })
}

/// Each of `count` runs of `script` in `sandbox`, in milliseconds altogether.
async fn timed(
    computers: &WorkerComputers,
    workspace: WorkspaceId,
    sandbox: Option<&Sandbox>,
    script: &str,
    count: u32,
) -> (Duration, String) {
    let started = Instant::now();
    let mut said = String::new();
    for _ in 0..count {
        said = match run(computers, workspace, sandbox.cloned(), script).await {
            Ok(Ok(_)) => "ok".to_owned(),
            Ok(Err(why)) => why,
            Err(why) => format!("{why}"),
        };
    }
    (started.elapsed(), said)
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one list of walls, read top to bottom"
)]
fn glave_s_sandbox_holds_on_a_persons_computer() {
    let (Ok(_), Ok(image), Ok("1"), Ok(class)) = (
        std::env::var("GLAVE_TEST_WORKER_GLAVE"),
        std::env::var("GLAVE_TEST_COMPUTER_IMAGE"),
        std::env::var("GLAVE_TEST_MEASURE").as_deref(),
        std::env::var("GLAVE_TEST_KUBE_RUNTIME_CLASS"),
    ) else {
        eprintln!("skipped: runs only where a person's computer is measured");
        return;
    };
    if !class.contains("gvisor") {
        eprintln!("skipped: bubblewrap makes no user namespace under {class}");
        return;
    }
    block_on!(async {
        let Some(cluster) = cluster(Some(&class), "sandboxed").await else {
            return;
        };
        let id = crate::local::new_id().unwrap().simple().to_string();
        let key = format!("sandboxed-{}", id.get(id.len() - 12..).unwrap());
        let person = PersonComputer::new(cluster.clone(), &key, 2).unwrap();
        person.sizing.wake_at(Size {
            cpu_millis: 2_000,
            memory_mib: 2_048,
        });
        let limits = Limits {
            isolation: cluster.isolation(),
            cpus: Some(2),
            memory_gb: Some(2),
            ..Limits::default()
        };
        let computers = WorkerComputers::on(Box::new(person), limits, WorkerCredentials::None);
        let environment = Environment {
            image: Image::Oci(image),
            network: Some(Network::On),
            ..Environment::host()
        };
        let woke = computers.create(&environment).await.unwrap();
        let prepared = ran(&computers, woke.id, PREPARE).await;
        let project = computers
            .open(
                woke.id,
                &Sources::Folder {
                    path: format!("{PERSON_HOME}/p"),
                },
            )
            .await
            .unwrap();
        let sandbox = |level, network| {
            Some(Sandbox {
                level,
                network,
                home: Some(PathBuf::from(PERSON_HOME)),
                glave_home: Some(PathBuf::from(format!("{PERSON_HOME}/.glave"))),
            })
        };
        let offline = sandbox(SandboxLevel::Project, false);
        let online = sandbox(SandboxLevel::Project, true);
        let reading = sandbox(SandboxLevel::ReadOnly, false);
        let py = |source: &str, args: &str| {
            format!("python3 -c '{}' {args}", source.replace('\'', "'\\''"))
        };
        let (internet, unix, netlink) = (
            py(SOCKET, "2 1.1.1.1 443"),
            py(SOCKET, "1 /opt/glave/glave.sock"),
            py(SOCKET, "16"),
        );
        let checks: Vec<(&str, Option<Sandbox>, String, Comes)> = vec![
            // Reads and writes.
            ("write beside the project", offline.clone(), "echo x > ~/outside".into(), Comes::Fails),
            ("write in the project", offline.clone(), "echo x > inside && cat inside".into(), Comes::Prints("x")),
            ("write in /tmp", offline.clone(), "echo x > /tmp/t && cat /tmp/t".into(), Comes::Prints("x")),
            ("write in its temp folder", offline.clone(), "echo x > \"$TMPDIR/t\" && cat \"$TMPDIR/t\"".into(), Comes::Prints("x")),
            ("read a file in the project", offline.clone(), "cat readme".into(), Comes::Prints("hello")),
            ("read the machine", offline.clone(), "cat /etc/os-release >/dev/null && echo read".into(), Comes::Prints("read")),
            ("read a key in the home", offline.clone(), "cat ~/.ssh/id_rsa".into(), Comes::Fails),
            ("read a project's .env", offline.clone(), "cat .env".into(), Comes::Fails),
            ("write a git hook", offline.clone(), "echo x > .git/hooks/pre-commit".into(), Comes::Fails),
            ("write git's config", offline.clone(), "echo x >> .git/config".into(), Comes::Fails),
            ("write glave's project settings", offline.clone(), "mkdir -p .glave/x".into(), Comes::Fails),
            ("commit", offline.clone(), "git add inside && git commit -qm two && echo committed".into(), Comes::Prints("committed")),
            ("write glave's program", offline.clone(), "echo x > /opt/glave/x".into(), Comes::Fails),
            // Whether it can be read, never what it holds, which would show in the test's output.
            ("read glave's token", offline.clone(), "test -r /opt/glave/secret/token && echo readable || echo hidden".into(), Comes::Prints("hidden")),
            ("see glave's folder", offline.clone(), "ls ~/.glave | grep -c disk.log || true".into(), Comes::Prints("0")),
            ("read-only level writes", reading.clone(), "echo x > other".into(), Comes::Fails),
            ("read-only level reads", reading.clone(), "cat readme".into(), Comes::Prints("hello")),
            // The network off, by the filter.
            ("offline: the internet", offline.clone(), internet.clone(), Comes::Prints("refused 1")),
            ("offline: glave's socket", offline.clone(), unix.clone(), Comes::Prints("refused 1")),
            ("offline: a netlink socket", offline.clone(), netlink.clone(), Comes::Prints("refused 1")),
            ("offline: curl", offline.clone(), "curl -s -m 5 -o /dev/null https://1.1.1.1".into(), Comes::Fails),
            ("offline: loopback", offline.clone(), py(LOOPBACK, ""), Comes::Fails),
            // The network on.
            ("online: the internet", online.clone(), internet, Comes::Prints("connected")),
            ("online: curl", online.clone(), "curl -s -m 20 -o /dev/null https://1.1.1.1 && echo fetched".into(), Comes::Prints("fetched")),
            ("online: loopback", online.clone(), py(LOOPBACK, ""), Comes::Prints("reached")),
            ("online: glave's socket", online.clone(), unix, Comes::Prints("refused 1")),
            ("online: a netlink socket", online.clone(), netlink, Comes::Prints("refused 1")),
            // What the filter refuses always; each errno is EPERM (1).
            ("ptrace", offline.clone(), py(SYSCALL, "101"), Comes::Prints("-1 1")),
            ("io_uring", offline.clone(), py(SYSCALL, "425"), Comes::Prints("-1 1")),
            // Processes of its own.
            ("see glave's process", offline.clone(), "for p in /proc/[0-9]*; do tr '\\0' ' ' <\"$p/cmdline\"; echo; done 2>/dev/null | grep -c \"computer [s]erve\" || true".into(), Comes::Prints("0")),
            ("no capabilities", offline.clone(), "awk '/^CapEff/ { print $2 }' /proc/self/status".into(), Comes::Prints("0000000000000000")),
        ];
        let mut wrong = Vec::new();
        let mut said = Vec::new();
        for (what, sandbox, script, comes) in checks {
            let answer = run(&computers, project.id, sandbox, &script).await;
            let holds = match (&answer, comes) {
                (Ok(Ok(text)), Comes::Prints(wanted)) => text == wanted,
                (Ok(Err(_)), Comes::Fails) => true,
                _ => false,
            };
            said.push(format!(
                "{}: {what}: {answer:?}",
                if holds { "holds" } else { "WRONG" }
            ));
            if !holds {
                wrong.push(what);
            }
        }
        // What it costs: a command's start, and work.
        let mut cost = Vec::new();
        for (what, script, count) in [
            ("20 empty commands", "true".to_owned(), 20),
            ("git status, 5 times", "git status --short".to_owned(), 5),
            (
                "compile Python's library",
                "rm -rf py && cp -r /usr/lib/python3.13 py && python3 -m compileall -q -f py >/dev/null".to_owned(),
                1,
            ),
        ] {
            for (under, sandbox) in [("none", None), ("sandbox", online.as_ref())] {
                let (took, ended) = timed(&computers, project.id, sandbox, &script, count).await;
                cost.push(format!("{what}, {under}: {} ms, {ended}", took.as_millis()));
            }
        }
        // Everyday work in the sandbox: each must pass with no one to ask.
        for (what, script, comes) in [
            (
                "pip",
                "python3 -m venv .venv && .venv/bin/pip -q install idna && .venv/bin/python -c 'import idna' && echo installed",
                "installed",
            ),
            (
                "uv",
                "uv venv -q .uvenv && uv pip install -q --python .uvenv/bin/python idna && echo installed",
                "installed",
            ),
            (
                "npm",
                "npm init -y >/dev/null 2>&1 && npm i --silent is-odd >/dev/null 2>&1 && echo installed",
                "installed",
            ),
            (
                "a toolchain",
                "rustup set profile minimal >/dev/null 2>&1 && rustup default stable >/dev/null 2>&1 && echo chosen",
                "chosen",
            ),
            (
                "a Rust program with a crate",
                "cargo new -q --vcs none dep 2>&1 && cd dep && cargo add -q itoa 2>&1 && cargo build -q 2>&1 && echo built",
                "built",
            ),
            (
                "a compiler",
                "printf 'int main(){return 0;}' > a.c && cc a.c -o a.out && ./a.out && echo compiled",
                "compiled",
            ),
        ] {
            let answer = run(&computers, project.id, online.clone(), script).await;
            let holds = matches!(&answer, Ok(Ok(text)) if text.ends_with(comes));
            said.push(format!(
                "{}: {what}: {answer:?}",
                if holds { "holds" } else { "WRONG" }
            ));
            if !holds {
                wrong.push(what);
            }
        }
        // None of it reached the homes commands outside the sandbox use.
        let outside = ran(
            &computers,
            woke.id,
            "ls -A ~/.cargo ~/.rustup 2>/dev/null | wc -l",
        )
        .await;
        let keeper_finds = ran(&computers, woke.id, "ls ~/.glave/sandbox-cache").await;
        let _ = computers.destroy(woke.id).await;
        let destroyed = PersonComputer::new(cluster.clone(), &key, 2)
            .unwrap()
            .destroy()
            .await;
        let left = left_behind(&cluster).await;
        eprintln!(
            "glave's sandbox under {class}\n{}\n\nwhat it costs\n{}\n\nsandbox cache: {keeper_finds:?}",
            said.join("\n"),
            cost.join("\n"),
        );
        assert_eq!(prepared, Ok("prepared".to_owned()));
        assert_eq!(
            outside,
            Ok("0".to_owned()),
            "the homes outside it stay empty"
        );
        assert_eq!(wrong, Vec::<&str>::new(), "every wall holds");
        destroyed.unwrap();
        assert_eq!(left, Vec::<String>::new());
    });
}
