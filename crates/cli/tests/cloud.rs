//! `glave --on cloud` end to end, against a real Docker Engine and a real worker: only with
//! `GLAVE_TEST_DOCKER=1`, `GLAVE_TEST_WORKER_GLAVE` (a Linux glave for the engine's processor;
//! CI's Linux job passes the one it just built) and `GLAVE_TEST_WORKER_IMAGE` (an image with
//! bash and git, pinned by digest). Elsewhere it says it skipped.
//!
//! The project's origin is served by `git daemon` with pushes allowed, at an address the
//! worker reaches (the Docker bridge's gateway on Linux, `host.docker.internal` on Docker
//! Desktop). The model writes a file on the worker, the session ends, the worker is released,
//! and its unpushed work arrives in origin on a `glave/wip/` branch.

#[cfg(test)]
#[path = "support/binary.rs"]
mod support;

#[cfg(test)]
mod cloud {
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time::Duration;

    use glave_ports::testing::Running;
    use glave_provider_openai::fake::{FakeServer, Reply};
    use serde_json::json;

    use super::support::*;

    fn run(dir: &Path, program: &str, args: &[&str]) -> String {
        let output = Command::new(program)
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{program} {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    /// The worker containers there are now.
    fn workers() -> Vec<String> {
        let listed = run(
            Path::new("."),
            "docker",
            &["ps", "-aq", "--filter", "label=glave.worker"],
        );
        listed.lines().map(str::to_owned).collect()
    }

    /// Where a container reaches this machine.
    fn host() -> String {
        if cfg!(target_os = "linux") {
            run(
                Path::new("."),
                "docker",
                &[
                    "network",
                    "inspect",
                    "bridge",
                    "--format",
                    "{{(index .IPAM.Config 0).Gateway}}",
                ],
            )
        } else {
            "host.docker.internal".to_owned()
        }
    }

    /// `git daemon` serving `base` on all addresses, pushes allowed, once it answers; stopped
    /// when dropped.
    fn daemon(base: &Path) -> (Running, u16) {
        let port = std::net::TcpListener::bind("0.0.0.0:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let child = Command::new("git")
            .args([
                "daemon",
                "--export-all",
                "--enable=receive-pack",
                "--reuseaddr",
                "--listen=0.0.0.0",
                &format!("--port={port}"),
                &format!("--base-path={}", base.display()),
                &base.display().to_string(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let running = Running::new(child);
        let url = format!("git://127.0.0.1:{port}/origin.git");
        for _ in 0..100 {
            let answered = Command::new("git")
                .args(["ls-remote", &url])
                .output()
                .is_ok_and(|output| output.status.success());
            if answered {
                return (running, port);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("git daemon did not start");
    }

    /// `project` as a repository with one commit, pushed to `origin.git` in a folder beside
    /// it: that folder, and the bare repository.
    fn pushed_project(project: &Path) -> (PathBuf, PathBuf) {
        let repos = project.with_file_name("repos");
        std::fs::create_dir_all(&repos).unwrap();
        let origin = repos.join("origin.git");
        let bare = origin.display().to_string();
        run(&repos, "git", &["init", "-q", "--bare", &bare]);
        run(project, "git", &["init", "-q", "--initial-branch=main"]);
        run(project, "git", &["add", "README.md"]);
        let who = ["-c", "user.name=t", "-c", "user.email=t@example.com"];
        let commit = [&who[..], &["commit", "-q", "-m", "one"]].concat();
        run(project, "git", &commit);
        run(project, "git", &["remote", "add", "origin", &bare]);
        run(project, "git", &["push", "-q", "-u", "origin", "main"]);
        (repos, origin)
    }

    #[test]
    fn a_thread_on_a_worker_pushes_its_work_when_released() {
        let wanted = |name| std::env::var(name).ok().filter(|value| !value.is_empty());
        let (Some(_), Some(linux_glave), Some(image)) = (
            wanted("GLAVE_TEST_DOCKER").filter(|on| on == "1"),
            wanted("GLAVE_TEST_WORKER_GLAVE"),
            wanted("GLAVE_TEST_WORKER_IMAGE"),
        ) else {
            eprintln!(
                "skipped: set GLAVE_TEST_DOCKER=1, GLAVE_TEST_WORKER_GLAVE (a Linux glave) and \
                 GLAVE_TEST_WORKER_IMAGE (an image with bash and git, pinned) for --on cloud"
            );
            return;
        };
        let place = place("cloud");
        let (repos, origin) = pushed_project(&place.project);
        let project = &place.project;
        let (daemon, port) = daemon(&repos);
        let url = format!("git://{}:{port}/origin.git", host());
        run(project, "git", &["remote", "set-url", "origin", &url]);
        std::fs::create_dir_all(project.join(".glave")).unwrap();
        std::fs::write(
            project.join(".glave/environment.toml"),
            format!("image = \"{image}\"\n"),
        )
        .unwrap();
        std::fs::create_dir_all(&place.home).unwrap();
        std::fs::write(
            place.home.join("config.toml"),
            format!(
                "on = \"cloud\"\n\n[workers]\nbackend = \"docker\"\nglave = \"{linux_glave}\"\n"
            ),
        )
        .unwrap();

        let before = workers();
        let server = FakeServer::start(vec![
            Reply::tool_call(
                "toolu_1",
                "write",
                &json!({"path": "notes.md", "content": "from the worker\n"}),
            ),
            Reply::text("I wrote notes.md."),
        ])
        .unwrap();
        let output = glave(&place, &server, &["--model", "m", "write notes"], "");
        drop(daemon);
        let shown = text(&output);
        assert!(output.status.success(), "{shown}");
        assert!(shown.contains("glave/wip/"), "{shown}");
        assert!(
            !project.join("notes.md").exists(),
            "the work happened on the worker"
        );
        let wip = run(
            &origin,
            "git",
            &[
                "for-each-ref",
                "--format=%(refname:short)",
                "refs/heads/glave/wip",
            ],
        );
        assert_eq!(wip.lines().count(), 1, "{wip}");
        let pushed = run(&origin, "git", &["show", &format!("{wip}:notes.md")]);
        assert_eq!(pushed, "from the worker");
        let sent = said(&server.received()[0].body, "user").join("\n");
        assert!(sent.contains("cloud worker"), "{sent}");
        assert_eq!(workers(), before, "the worker is removed");
    }
}
