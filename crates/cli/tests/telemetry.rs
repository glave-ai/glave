//! Logs and telemetry from the real binary: `--log` records turns, model calls, tool calls and
//! appends with their counts and timings, and never the content: not the prompt, the answer,
//! a tool's arguments or output, a file, or the key. With the `otlp` feature, the same holds
//! for what reaches an OpenTelemetry collector.

#[cfg(test)]
#[path = "support/binary.rs"]
mod support;

#[cfg(test)]
mod telemetry {
    use glave_ports::testing::FAKE_SECRET;
    use glave_provider_openai::fake::{FakeServer, Reply};
    use serde_json::json;

    use super::support::*;

    /// Strings that are content, by what they are: none may reach a log or a collector.
    const CONTENT: [(&str, &str); 5] = [
        ("the prompt", "PROMPT-c0ffee"),
        ("a tool's arguments", "ARGUMENT-5eed"),
        ("a file", "FILE-CONTENT-b1ade"),
        ("the answer", "ANSWER-f00d"),
        ("the key", FAKE_SECRET),
    ];

    /// The model's side of a turn that writes a file and reads it back with a command.
    fn replies() -> Vec<Reply> {
        vec![
            Reply::tool_call(
                "toolu_1",
                "write",
                &json!({"path": "ARGUMENT-5eed.md", "content": "FILE-CONTENT-b1ade\n"}),
            ),
            Reply::tool_call(
                "toolu_2",
                "bash",
                &json!({"command": "cat ARGUMENT-5eed.md"}),
            ),
            Reply::text("ANSWER-f00d"),
        ]
    }

    #[test]
    fn the_log_has_spans_and_metrics_and_never_content() {
        let place = place("log");
        let server = FakeServer::start(replies()).unwrap();
        let log = place.home.join("logs/glave.log");
        let log_arg = log.to_string_lossy().into_owned();
        let mut child = start_with(
            &place,
            &server,
            &[
                "--model",
                "fake-model",
                "--log",
                &log_arg,
                "do it PROMPT-c0ffee",
            ],
            &[("GLAVE_LOG", "trace")],
        );
        drop(child.stdin.take());
        let output = child.wait_with_output().unwrap();
        let shown = text(&output);
        assert!(output.status.success(), "{shown}");
        assert!(shown.contains("ANSWER-f00d"), "{shown}");
        let first = &server.received()[0].body;
        assert!(said(first, "user")[0].contains("PROMPT-c0ffee"));
        let written = std::fs::read_to_string(&log).unwrap();
        for span in [
            "session.request",
            "turn",
            "model.call",
            "tool.call",
            "store.append",
        ] {
            assert!(written.contains(span), "no {span} in the log:\n{written}");
        }
        for metric in [
            "monotonic_counter.glave.turns",
            "monotonic_counter.glave.model.input_tokens",
            "monotonic_counter.glave.tool.calls",
        ] {
            assert!(
                written.contains(metric),
                "no {metric} in the log:\n{written}"
            );
        }
        for (what, content) in CONTENT {
            assert!(!written.contains(content), "{what} in the log:\n{written}");
        }
    }

    #[test]
    fn a_log_level_that_is_not_one_is_refused() {
        let place = place("log-level");
        let server = FakeServer::start(vec![Reply::text("hi")]).unwrap();
        let log = place.home.join("glave.log");
        let log_arg = log.to_string_lossy().into_owned();
        let child = start_with(
            &place,
            &server,
            &["--model", "fake-model", "--log", &log_arg, "hi"],
            &[("GLAVE_LOG", "loud")],
        );
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success());
        assert!(text(&output).contains("GLAVE_LOG"), "{}", text(&output));
    }

    #[cfg(not(feature = "otlp"))]
    #[test]
    fn a_collector_without_the_feature_is_refused_naming_it() {
        let place = place("no-otlp");
        std::fs::create_dir_all(&place.home).unwrap();
        std::fs::write(
            place.home.join("config.toml"),
            "[telemetry]\notlp = \"http://127.0.0.1:9\"\n",
        )
        .unwrap();
        let server = FakeServer::start(vec![Reply::text("hi")]).unwrap();
        let output = glave(&place, &server, &["--model", "fake-model", "hi"], "");
        assert!(!output.status.success());
        assert!(
            text(&output).contains("--features otlp"),
            "{}",
            text(&output)
        );
    }

    #[cfg(feature = "otlp")]
    #[test]
    fn a_collector_gets_spans_and_metrics_and_never_content() {
        let collector = collector::Collector::start();
        let place = place("otlp");
        std::fs::create_dir_all(&place.home).unwrap();
        std::fs::write(
            place.home.join("config.toml"),
            format!("[telemetry]\notlp = \"{}\"\n", collector.url()),
        )
        .unwrap();
        let server = FakeServer::start(replies()).unwrap();
        let output = glave(
            &place,
            &server,
            &["--model", "fake-model", "do it PROMPT-c0ffee"],
            "",
        );
        assert!(output.status.success(), "{}", text(&output));
        let posts = collector.posts();
        let paths: Vec<&str> = posts.iter().map(|(path, _)| path.as_str()).collect();
        assert!(paths.contains(&"/v1/traces"), "{paths:?}");
        assert!(paths.contains(&"/v1/metrics"), "{paths:?}");
        let bodies: Vec<u8> = posts.into_iter().flat_map(|(_, body)| body).collect();
        let bodies = String::from_utf8_lossy(&bodies);
        for name in ["turn", "model.call", "tool.call", "glave.turns"] {
            assert!(bodies.contains(name), "no {name} exported");
        }
        for (what, content) in CONTENT {
            assert!(!bodies.contains(content), "{what} exported");
        }
    }

    #[cfg(feature = "otlp")]
    mod collector {
        //! A stand-in OpenTelemetry collector: every POST's path and body, answered 200.

        use std::io::{BufRead as _, BufReader, Read as _, Write as _};
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};

        type Posts = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

        pub struct Collector {
            url: String,
            posts: Posts,
        }

        impl Collector {
            pub fn start() -> Self {
                let listener = TcpListener::bind("127.0.0.1:0").unwrap();
                let url = format!("http://{}", listener.local_addr().unwrap());
                let posts = Posts::default();
                let kept = Arc::clone(&posts);
                std::thread::spawn(move || {
                    for stream in listener.incoming() {
                        let Ok(mut stream) = stream else { return };
                        let mut reader = BufReader::new(stream.try_clone().unwrap());
                        let mut line = String::new();
                        reader.read_line(&mut line).unwrap();
                        let path = line.split(' ').nth(1).unwrap_or_default().to_owned();
                        let mut length = 0;
                        loop {
                            let mut header = String::new();
                            reader.read_line(&mut header).unwrap();
                            let header = header.trim().to_ascii_lowercase();
                            if header.is_empty() {
                                break;
                            }
                            if let Some(value) = header.strip_prefix("content-length:") {
                                length = value.trim().parse().unwrap();
                            }
                        }
                        let mut body = vec![0; length];
                        reader.read_exact(&mut body).unwrap();
                        kept.lock().unwrap().push((path, body));
                        let _ = stream.write_all(
                            b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                        );
                    }
                });
                Self { url, posts }
            }

            pub fn url(&self) -> &str {
                &self.url
            }

            pub fn posts(&self) -> Vec<(String, Vec<u8>)> {
                self.posts.lock().unwrap().clone()
            }
        }
    }
}
