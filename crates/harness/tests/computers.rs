//! The same turn on this machine and on a computer across the wire: a remote computer served
//! over TCP on 127.0.0.1 must give the model exactly what a local one does, tool for tool.

mod golden {
    use std::path::Path;
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};

    use crate::support::*;
    use crate::wire::across_the_wire;
    use glave_computer::LocalComputers;
    use glave_event::{Event, StopReason};
    use glave_harness::{AllowAll, Workspace};
    use glave_ports::{Completion, InMemoryLog};
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
    async fn played(workspace: Workspace) -> (Vec<Event>, String) {
        let (harness, seen) = harness_with(
            workspace,
            script(),
            AllowAll,
            InMemoryLog::new(),
            1,
            settings(),
        );
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "tidy up").await;
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
        block_on!(async {
            project(&dir);
            let (local_events, local_seen) = played(workspace(&dir).await).await;

            project(&dir);
            let copies = dir.with_extension("copies");
            let computers = Arc::new(LocalComputers::new(&copies, std::env::vars_os()));
            let (remote_workspace, server) = across_the_wire(computers, &dir).await;
            let (remote_events, remote_seen) = played(remote_workspace).await;
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

mod credentials {
    use std::sync::Arc;

    use crate::support::*;
    use glave_event::{Body, StopReason};
    use glave_harness::AllowAll;
    use glave_ports::InMemoryLog;
    use glave_ports::computer::{CredentialHost, Forge, GitCredential};
    use glave_ports::testing::{FAKE_COMMIT, FakeGitHost, FixedGitCredentials, fake_secret};
    use serde_json::json;

    /// With hosts connected, the model is told what is signed in, never a token, and the
    /// commands it runs sign in through the harness's credentials.
    #[test]
    fn the_model_is_told_what_is_signed_in_and_its_commands_sign_in() {
        let dir = folder("signed-in");
        let token = fake_secret("ghp_");
        let fake = FakeGitHost::start("x-access-token", &token).unwrap();
        let hosts = vec![
            CredentialHost {
                url: "https://github.com".into(),
                forge: Forge::Github,
            },
            CredentialHost {
                url: fake.url.clone(),
                forge: Forge::Other,
            },
        ];
        let credential = GitCredential::new("x-access-token", &token).unwrap();
        let credentials = Arc::new(FixedGitCredentials::new(hosts, credential));
        let fetch = format!(
            "GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null GIT_TERMINAL_PROMPT=0 \
             git ls-remote {}/repo.git",
            fake.url
        );
        block_on!(async {
            let answers = vec![
                answer(
                    None,
                    vec![("bash", json!({"command": fetch}))],
                    StopReason::ToolUse,
                ),
                answer(Some("Done."), vec![], StopReason::EndTurn),
            ];
            let workspace = workspace(&dir).await.with_credentials(credentials);
            let (harness, seen) = harness_with(
                workspace,
                answers,
                AllowAll,
                InMemoryLog::new(),
                1,
                settings(),
            );
            let mut thread = harness.start_thread(scope()).await.unwrap();
            turn(&harness, &mut thread, "list the branches").await;
            let system = seen.lock().unwrap()[0].system.clone();
            assert!(
                system.contains("`gh` is signed in for github.com") && !system.contains(&token),
                "{system}"
            );
            let result = events(harness.log(), thread.id)
                .await
                .into_iter()
                .find_map(|event| match event.body {
                    Body::ToolCallCompleted(done) => Some(format!("{done:?}")),
                    _ => None,
                })
                .unwrap();
            assert!(result.contains(FAKE_COMMIT), "{result}");
            assert!(!result.contains(&token), "{result}");
        });
    }
}

mod secrets {
    use glave_event::{Body, Secret, StopReason};
    use glave_harness::AllowAll;
    use glave_ports::InMemoryLog;
    use glave_ports::secrets::{Known, KnownSecrets};
    use glave_ports::testing::any_secret;
    use serde_json::json;

    use crate::support::*;

    /// Commands see a secret's placeholder in its variable, and its value, where output shows
    /// it anyway, is masked before it is recorded or the model is shown it.
    #[test]
    fn commands_see_placeholders_and_a_value_in_output_is_masked() {
        let dir = folder("secrets");
        let value = any_secret();
        std::fs::write(dir.join("echoed.txt"), format!("the api said {value}\n")).unwrap();
        let known = KnownSecrets::default();
        let secret = Known::new("STRIPE_KEY".into(), Secret::new(value.clone()));
        let placeholder = secret.placeholder.clone();
        known.set(vec![secret]);
        let command = r#"printf 'key=%s\n' "$STRIPE_KEY" && cat echoed.txt"#;
        block_on!(async {
            let answers = vec![
                answer(
                    None,
                    vec![("bash", json!({"command": command}))],
                    StopReason::ToolUse,
                ),
                answer(Some("Done."), vec![], StopReason::EndTurn),
            ];
            let workspace = workspace(&dir).await.with_secrets(known);
            let log = InMemoryLog::new();
            let (harness, seen) = harness_with(workspace, answers, AllowAll, log, 1, settings());
            let mut thread = harness.start_thread(scope()).await.unwrap();
            turn(&harness, &mut thread, "call the api").await;
            let result = events(harness.log(), thread.id)
                .await
                .into_iter()
                .find_map(|event| match event.body {
                    Body::ToolCallCompleted(done) => Some(format!("{done:?}")),
                    _ => None,
                })
                .unwrap();
            assert!(result.contains(&format!("key={placeholder}")), "{result}");
            assert!(result.contains("the api said [redacted]"), "{result}");
            let all = format!("{:?}", events(harness.log(), thread.id).await);
            assert!(!all.contains(&value), "a value reached the log: {all}");
            let shown = format!("{:?}", seen.lock().unwrap());
            assert!(!shown.contains(&value), "the model saw a value: {shown}");
        });
    }
}
