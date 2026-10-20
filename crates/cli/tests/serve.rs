//! `glave serve`, the real binary. Over stdio, driven the way another program drives it:
//! JSON-RPC 2.0 requests on its standard input, one per line, and the answers and a turn's
//! notifications on its standard output. Over HTTP, the web app beside the API: the one built
//! into glave (the `web` feature) with no folder named, under the same policy as a folder's,
//! and `--web`'s folder rather than it; without the feature and a folder, no app, as before.
//! And a server a test started is gone when the test ends, by failing too.

mod serving {
    use std::io::{BufRead as _, BufReader, Write as _};

    use glave_provider_openai::fake::{FakeServer, Reply};
    use serde_json::{Value, json};

    use crate::support::*;

    #[test]
    fn another_program_runs_a_turn_over_stdio() {
        let place = place("serve");
        let server = FakeServer::start(vec![Reply::text("Hello from glave.")]).unwrap();
        let mut child = start(&place, &server, &["serve", "--stdio", "--model", "m"]);
        let mut input = child.stdin.take().unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut send = |message: Value| {
            writeln!(input, "{message}").unwrap();
            input.flush().unwrap();
        };
        let mut answer_to = |id: i64, notes: &mut Vec<Value>| loop {
            let message: Value = serde_json::from_str(&output.next().unwrap().unwrap()).unwrap();
            assert_eq!(message["jsonrpc"], "2.0");
            if message["id"] == id {
                return message;
            }
            notes.push(message);
        };
        let mut notes = Vec::new();

        send(json!({"jsonrpc": "2.0", "id": 1, "method": "server/discover"}));
        let discovered = answer_to(1, &mut notes);
        assert_eq!(discovered["result"]["name"], "glave");

        send(json!({"jsonrpc": "2.0", "id": 2, "method": "thread/start"}));
        let thread = answer_to(2, &mut notes)["result"]["thread"]["id"].clone();
        assert!(thread.is_string());

        send(json!({"jsonrpc": "2.0", "id": 3, "method": "turn/start",
                    "params": {"thread": thread, "prompt": "hi"}}));
        let turn = answer_to(3, &mut notes);
        assert_eq!(turn["result"]["turn"]["outcome"], "completed");
        assert!(
            notes.iter().any(|note| note["method"] == "turn/live"
                && note["params"]["update"]["text"] == "Hello from glave."),
            "{notes:?}"
        );
        assert!(notes.iter().any(|note| note["method"] == "thread/event"
            && note["params"]["event"]["type"] == "turn.ended"));

        send(json!({"jsonrpc": "2.0", "id": 4, "method": "thread/explode"}));
        assert_eq!(answer_to(4, &mut notes)["error"]["code"], -32601);

        drop(input);
        let status = child.wait().unwrap();
        assert!(status.success(), "glave ends when its input does");
    }
}

mod web {
    use glave_ports::block_on;
    use glave_provider_openai::fake::FakeServer;

    use crate::http::serve_http;
    use crate::support::*;

    /// The nonce a Content-Security-Policy admits styles with.
    fn nonce_of(policy: &str) -> String {
        let (_, after) = policy.split_once("'nonce-").unwrap();
        after.split('\'').next().unwrap().to_owned()
    }

    #[test]
    fn the_app_built_into_glave_is_served_with_no_folder_named() {
        let place = place("serve-built-in");
        let model = FakeServer::start(vec![]).unwrap();
        let (_served, url) = serve_http(&place, &model, &[]);
        block_on!(async {
            if cfg!(feature = "web") {
                the_app_is_served(&url).await;
            } else {
                let page = reqwest::get(format!("{url}/")).await.unwrap();
                assert_eq!(page.status(), 405, "no app, only the API");
            }
        });
    }

    /// The built app: a route is its page with this load's nonce, under the policy; its script
    /// is cached for good; an asset it does not have is not found.
    async fn the_app_is_served(url: &str) {
        let page = reqwest::get(format!("{url}/t/123")).await.unwrap();
        assert_eq!(page.status(), 200);
        let headers = page.headers().clone();
        assert_eq!(headers["content-type"], "text/html; charset=utf-8");
        assert_eq!(headers["cache-control"], "no-store");
        assert_eq!(headers["x-frame-options"], "DENY");
        let policy = headers["content-security-policy"].to_str().unwrap();
        assert!(
            policy.contains("require-trusted-types-for 'script'"),
            "{policy}"
        );
        let nonce = nonce_of(policy);
        let body = page.text().await.unwrap();
        assert!(!body.contains("GLAVE_CSP_NONCE"), "{body}");
        assert!(body.contains(&format!("ngcspnonce=\"{nonce}\"")), "{body}");

        let (_, after) = body.split_once("src=\"main-").unwrap();
        let script = format!("main-{}", after.split('"').next().unwrap());
        let main = reqwest::get(format!("{url}/{script}")).await.unwrap();
        assert_eq!(main.status(), 200, "{script}");
        assert_eq!(
            main.headers()["content-type"],
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            main.headers()["cache-control"],
            "public, max-age=31536000, immutable"
        );
        assert!(!main.bytes().await.unwrap().is_empty());

        let missing = reqwest::get(format!("{url}/main-ABCD1234.js"))
            .await
            .unwrap();
        assert_eq!(missing.status(), 404);
    }

    #[test]
    fn torn_token_and_address_files_are_written_anew_at_start_and_work() {
        let place = place("serve-torn");
        std::fs::create_dir_all(&place.home).unwrap();
        // Parts of each, as a crash while writing them would leave.
        std::fs::write(place.home.join("serve.token"), "glave_loc").unwrap();
        std::fs::write(place.home.join("serve.url"), "http://127.0").unwrap();
        let model = FakeServer::start(vec![]).unwrap();
        let (_served, url) = serve_http(&place, &model, &[]);
        let token = std::fs::read_to_string(place.home.join("serve.token")).unwrap();
        assert_eq!(token.len(), "glave_local_".len() + 64, "{token}");
        let kept = std::fs::read_to_string(place.home.join("serve.url")).unwrap();
        assert_eq!(kept, url);
        block_on!(async {
            let client = reqwest::Client::new();
            let ask = |token: String| {
                client
                    .post(format!("{kept}/"))
                    .bearer_auth(token)
                    .header("content-type", "application/json")
                    .body(r#"{"jsonrpc":"2.0","id":1,"method":"thread/list","params":{}}"#)
                    .send()
            };
            let answered = ask(token).await.unwrap();
            assert_eq!(answered.status(), 200);
            let body = answered.text().await.unwrap();
            assert!(body.contains(r#""result""#), "{body}");
            let refused = ask("glave_loc".into()).await.unwrap();
            assert_eq!(refused.status(), 401, "the torn token signs nothing in");
        });
    }

    #[test]
    fn a_web_folder_is_served_rather_than_the_app_built_in() {
        let place = place("serve-web-folder");
        let folder = place.project.join("../browser");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("index.html"),
            r#"<p>From the folder.</p><app-root ngCspNonce="GLAVE_CSP_NONCE"></app-root>"#,
        )
        .unwrap();
        let model = FakeServer::start(vec![]).unwrap();
        let (_served, url) = serve_http(&place, &model, &["--web", folder.to_str().unwrap()]);
        block_on!(async {
            let page = reqwest::get(format!("{url}/t/123")).await.unwrap();
            assert_eq!(page.status(), 200);
            let policy = page.headers()["content-security-policy"].to_str().unwrap();
            let nonce = nonce_of(policy);
            let body = page.text().await.unwrap();
            assert!(body.starts_with("<p>From the folder.</p>"), "{body}");
            assert!(body.contains(&nonce), "{body}");
        });
    }
}

mod stopped {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::process::Command;

    use glave_provider_openai::fake::FakeServer;

    use crate::http::serve_http;
    use crate::support::*;

    /// Whether the process `id` is still there.
    fn there(id: u32) -> bool {
        Command::new("kill")
            .args(["-0", &id.to_string()])
            .status()
            .unwrap()
            .success()
    }

    #[test]
    fn a_server_a_test_started_is_gone_when_the_test_ends_even_by_failing() {
        let place = place("serve-stopped");
        let model = FakeServer::start(vec![]).unwrap();
        let (served, _) = serve_http(&place, &model, &[]);
        let id = served.id();
        assert!(there(id));
        drop(served);
        assert!(!there(id), "gone once dropped");

        let mut id = None;
        let failed = catch_unwind(AssertUnwindSafe(|| {
            let (served, _) = serve_http(&place, &model, &[]);
            id = Some(served.id());
            panic!("the test fails while its server runs");
        }));
        assert!(failed.is_err());
        assert!(!there(id.unwrap()), "gone once the failed test unwound");
    }
}

mod reached {
    use glave_ports::block_on;
    use glave_provider_openai::fake::FakeServer;

    use crate::http::serve_http;
    use crate::support::*;

    const DISCOVER: &str = r#"{"jsonrpc":"2.0","id":1,"method":"server/discover"}"#;

    /// The status of `server/discover` at `url`, sent to the name `host` from a page at
    /// `page`, as a tunnel on this computer forwards a browser's request.
    async fn discovered(url: &str, host: &str, page: &str) -> u16 {
        reqwest::Client::new()
            .post(format!("{url}/"))
            .header("host", host)
            .header("origin", page)
            .header("content-type", "application/json")
            .body(DISCOVER)
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    }

    #[test]
    fn a_tunnel_here_reaches_glave_at_its_public_origin_and_no_other_name() {
        let place = place("serve-origin");
        let model = FakeServer::start(vec![]).unwrap();
        let public = "https://glave.example.com";
        let (served, url) = serve_http(&place, &model, &[]);
        block_on!(async {
            assert_eq!(discovered(&url, "glave.example.com", public).await, 403);
        });
        drop(served);
        let origin = format!("{{ origin = \"{public}\" }}");
        choose(&place, "serve", &origin);
        // First again, as the server's own settings begin.
        choose(&place, "titles", "false");
        let (_served, url) = serve_http(&place, &model, &[]);
        block_on!(async {
            assert_eq!(discovered(&url, "glave.example.com", public).await, 200);
            let elsewhere = discovered(&url, "other.example.com", public).await;
            assert_eq!(elsewhere, 403);
            let page = discovered(&url, "glave.example.com", "https://evil.example.com").await;
            assert_eq!(page, 403);
        });
    }

    #[test]
    fn beyond_this_computer_glave_listens_only_behind_https() {
        let place = place("serve-exposed");
        let model = FakeServer::start(vec![]).unwrap();
        let args = [
            "serve",
            "--http",
            "0.0.0.0:0",
            "--issuer",
            "https://id.example.com",
            "--audience",
            "glave",
        ];
        let refused = glave(&place, &model, &args, "");
        assert!(!refused.status.success());
        let said = text(&refused);
        assert!(said.contains("nothing serving HTTPS"), "{said}");
        assert!(said.contains("origin under [serve]"), "{said}");
    }
}
