//! The same turn on this machine and on a computer across the wire: a remote computer served
//! over TCP on 127.0.0.1 must give the model exactly what a local one does, tool for tool.

#[cfg(test)]
#[path = "support/script.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod golden {
    use std::path::Path;
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};

    use super::support::*;
    use glave_computer::{LocalComputers, RemoteComputers, serve_tcp};
    use glave_event::{Event, StopReason};
    use glave_harness::{AllowAll, Cancel, Workspace};
    use glave_ports::{Completion, Computers, Environment, MemoryLog, Sources};
    use serde_json::json;

    /// The project both runs start from. Times are fixed, so `glob`'s newest-first order is too.
    fn project(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
        let base = SystemTime::UNIX_EPOCH + Duration::from_hours(500_000);
        let files = [
            (".gitignore", "target/\n"),
            ("src/lib.rs", "fn alpha() {}\nfn beta() {}\n"),
            ("src/other.rs", "fn gamma() {}\n"),
            ("target/built.rs", "fn alpha() {}\n"),
            (".env", "ALPHA=secret\n"),
            ("README.md", "# demo\n"),
        ];
        for (minute, (path, content)) in (0_u64..).zip(files) {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, content).unwrap();
            let file = std::fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(base + Duration::from_mins(minute))
                .unwrap();
        }
    }

    /// One call per step, so the order of the results never depends on timing.
    fn script() -> Vec<Completion> {
        let calls = [
            ("read", json!({"path": "src/lib.rs"})),
            ("read", json!({"path": "."})),
            ("glob", json!({"pattern": "*.rs"})),
            ("search", json!({"pattern": "fn \\w+"})),
            ("search", json!({"pattern": "alpha(", "glob": "*.rs"})),
            (
                "edit",
                json!({"path": "src/lib.rs", "old_string": "fn beta() {}", "new_string": "fn beta() { alpha() }"}),
            ),
            (
                "write",
                json!({"path": "notes/new.txt", "content": "one\ntwo\n"}),
            ),
            (
                "write",
                json!({"path": "src/other.rs", "content": "unread\n"}),
            ),
            (
                "edit",
                json!({"path": "missing.rs", "old_string": "a", "new_string": "b"}),
            ),
            ("read", json!({"path": "../outside.txt"})),
            ("glob", json!({"pattern": "["})),
            ("bash", json!({"command": "cat notes/new.txt && ls src"})),
            ("bash", json!({"command": "echo failing >&2; exit 3"})),
        ];
        let mut answers: Vec<Completion> = calls
            .into_iter()
            .map(|(tool, args)| answer(None, vec![(tool, args)], StopReason::ToolUse))
            .collect();
        answers.push(answer(Some("Done."), vec![], StopReason::EndTurn));
        answers
    }

    /// Runs the scripted turn on `workspace`, returning every event and what the model saw.
    async fn turn(workspace: Workspace) -> (Vec<Event>, String) {
        let (harness, seen) = harness_with(
            workspace,
            script(),
            AllowAll,
            MemoryLog::new(),
            1,
            settings(),
        );
        let mut thread = harness.start_thread(scope()).await.unwrap();
        harness
            .run_turn(&mut thread, "tidy up", &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        let events = events(harness.log(), thread.id).await;
        let last = seen.lock().unwrap().last().cloned().unwrap();
        (events, without_timings(&format!("{last:?}")))
    }

    /// Command footers say how long a command took (`[exit 0 · 0.1 s]`), which varies.
    fn without_timings(text: &str) -> String {
        let mut pieces = text.split(" · ");
        let mut out = pieces.next().unwrap_or_default().to_owned();
        for piece in pieces {
            out.push_str(" · N");
            match piece.split_once(" s]") {
                Some((_, rest)) => {
                    out.push_str(" s]");
                    out.push_str(rest);
                }
                None => out.push_str(piece),
            }
        }
        out
    }

    #[test]
    fn a_remote_computer_gives_the_model_exactly_what_a_local_one_does() {
        let dir = folder("computers-golden");
        block_on(async {
            project(&dir);
            let (local_events, local_seen) = turn(workspace(&dir).await).await;

            project(&dir);
            let copies = dir.with_extension("copies");
            let computers = Arc::new(LocalComputers::new(&copies, std::env::vars_os()));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(serve_tcp(computers, listener, "token".into()));
            let stream = tokio::net::TcpStream::connect(address).await.unwrap();
            let (read, write) = stream.into_split();
            let remote = RemoteComputers::connect(read, write);
            remote.authenticate("token").await.unwrap();
            remote.check_protocol().await.unwrap();
            let computer = remote.create(&Environment::host()).await.unwrap();
            let sources = Sources::Folder {
                path: dir.to_string_lossy().into_owned(),
            };
            let info = remote.open(computer.id, &sources).await.unwrap();
            let remote_workspace = Workspace::new(Arc::new(remote), info);
            let (remote_events, remote_seen) = turn(remote_workspace).await;
            server.abort();

            assert_eq!(remote_seen, local_seen);
            assert!(
                local_seen.contains("src/lib.rs:2:fn beta() {}"),
                "{local_seen}"
            );
            assert!(local_seen.contains("outside the workspace"), "{local_seen}");
            assert_eq!(kinds(&remote_events), kinds(&local_events));
            assert_eq!(
                std::fs::read_to_string(dir.join("src/lib.rs")).unwrap(),
                "fn alpha() {}\nfn beta() { alpha() }\n"
            );
        });
    }
}
