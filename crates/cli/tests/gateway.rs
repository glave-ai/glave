//! The gateway as a service, end to end with the real binary: `glave gateway serve` holds the
//! model provider's key and the organization's tool catalog, and `glave` with `[gateway] url`
//! sends its model calls there, signed in as the person, and gets the catalog's tools. Without
//! `[gateway]`, glave reaches the model directly, exactly as before.

use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::SystemTime;

use glave_ports::testing::{FAKE_SECRET, Running};
use glave_ports::{Clock as _, SystemClock};
use glave_provider_openai::fake::{FakeServer, Reply};
use serde_json::json;

use crate::issuer::{AUDIENCE, Issuer, Signer};
use crate::support::*;

/// The key only the gateway holds.
const KEY: &str = FAKE_SECRET;

/// The fake identity provider, served from a thread of its own, and the key it signs with.
struct Identity {
    url: String,
    signer: Arc<Signer>,
}

impl Identity {
    fn start() -> Self {
        let signer = Arc::new(Signer::es256("k1"));
        let publishing = Arc::clone(&signer);
        let (sender, url) = mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let issuer = Issuer::start().await;
                issuer.publish(&publishing);
                sender.send(issuer.url.clone()).unwrap();
                // Serves until the test process ends.
                std::future::pending::<()>().await;
            });
        });
        Self {
            url: url.recv().unwrap(),
            signer,
        }
    }

    /// A token for `subject`, good for ten minutes from now.
    fn token(&self, subject: &str) -> String {
        let now = SystemTime::from(SystemClock.now())
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        self.signer.sign(&json!({
            "iss": self.url, "sub": subject, "aud": AUDIENCE,
            "iat": now, "exp": now + 600,
        }))
    }
}

/// `glave gateway serve`, stopped when dropped.
struct Gateway {
    _running: Running,
    url: String,
    /// What it has said on its standard error so far.
    said: Arc<Mutex<Vec<String>>>,
}

fn test_server() -> PathBuf {
    Path::new(env!("CARGO_BIN_EXE_glave")).with_file_name("glave-mcp-test-server")
}

/// The gateway over `upstream`, offering `docs` of the two servers in its `mcp.json`.
fn serve(place: &Place, identity: &Identity, upstream: &FakeServer) -> Gateway {
    let home = place.project.join("../gateway-home");
    std::fs::create_dir_all(&home).unwrap();
    let servers = json!({"mcpServers": {
        "docs": {"command": test_server()},
        "tracker": {"command": test_server()},
    }});
    std::fs::write(home.join("mcp.json"), servers.to_string()).unwrap();
    let policy = home.join("policy.toml");
    std::fs::write(&policy, "version = 1\n[mcp]\ncatalog = [\"docs\"]\n").unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_glave"))
        .args(["gateway", "serve", "--http", "127.0.0.1:0"])
        .args(["--issuer", &identity.url, "--audience", AUDIENCE])
        .arg("--policy")
        .arg(&policy)
        .env("GLAVE_HOME", &home)
        .env("GLAVE_BASE_URL", upstream.url())
        .env("GLAVE_API_KEY", KEY)
        .env("GLAVE_MODEL", "big")
        .env_remove("GLAVE_JUDGE_MODEL")
        .env_remove("GLAVE_PROVIDER")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut running = Running::new(child);
    let mut lines = BufReader::new(running.stderr.take().unwrap()).lines();
    let said = Arc::new(Mutex::new(Vec::new()));
    let url = loop {
        let line = lines.next().unwrap().unwrap();
        said.lock().unwrap().push(line.clone());
        if let Some(url) = line.strip_prefix("glave: serving the gateway on ") {
            break url.to_owned();
        }
    };
    // Whatever it says later is still read, so it never blocks.
    let saying = Arc::clone(&said);
    std::thread::spawn(move || {
        for line in lines.map_while(Result::ok) {
            saying.lock().unwrap().push(line);
        }
    });
    Gateway {
        _running: running,
        url,
        said,
    }
}

#[test]
fn with_a_gateway_url_model_calls_and_the_catalog_come_through_the_gateway() {
    let place = place("gateway-client");
    let identity = Identity::start();
    let upstream = FakeServer::start(vec![
        Reply::tool_call(
            "call_1",
            "mcp__docs__echo",
            &json!({"text": "via the catalog"}),
        ),
        Reply::text("Done."),
    ])
    .unwrap();
    let gateway = serve(&place, &identity, &upstream);
    std::fs::create_dir_all(&place.home).unwrap();
    std::fs::write(
        place.home.join("config.toml"),
        format!("sandbox = \"off\"\n[gateway]\nurl = \"{}\"\n", gateway.url),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_glave"))
        // Unconfined and without asking, so the call runs wherever the sandbox cannot.
        .args(["--approval", "auto"])
        .arg("echo it through the catalog")
        .current_dir(&place.project)
        .env("GLAVE_HOME", &place.home)
        .env("GLAVE_GATEWAY_TOKEN", identity.token("alice"))
        .env("NO_COLOR", "1")
        .env_remove("GLAVE_API_KEY")
        .env_remove("GLAVE_BASE_URL")
        .env_remove("GLAVE_MODEL")
        .env_remove("GLAVE_PROVIDER")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let shown = text(&output);
    assert!(output.status.success(), "{shown}");
    assert!(shown.contains("Done."), "{shown}");
    let received = upstream.received();
    assert_eq!(received.len(), 2, "{shown}");
    let first = &received[0];
    assert_eq!(first.body["model"], "big", "the gateway's main route");
    assert_eq!(
        first.authorization.as_deref(),
        Some(&*format!("Bearer {KEY}")),
        "the key is the gateway's own"
    );
    let offered: Vec<&str> = first.body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap())
        .collect();
    assert!(offered.contains(&"mcp__docs__echo"), "{offered:?}");
    assert!(
        !offered
            .iter()
            .any(|name| name.starts_with("mcp__tracker__")),
        "only the catalog the policy names: {offered:?}"
    );
    let results = said(&received[1].body, "tool");
    assert!(results[0].contains("via the catalog"), "{results:?}");
    assert!(!shown.contains(KEY), "{shown}");
    let logged = gateway.said.lock().unwrap().join("\n");
    assert!(!logged.contains(KEY), "{logged}");
}

#[test]
fn without_a_gateway_url_glave_reaches_the_model_directly_as_before() {
    let place = place("gateway-none");
    std::fs::create_dir_all(&place.home).unwrap();
    std::fs::write(place.home.join("config.toml"), "memory = \"off\"\n").unwrap();
    let server = FakeServer::start(vec![Reply::text("Direct.")]).unwrap();
    let output = glave(&place, &server, &["--model", "m", "hello"], "");
    let shown = text(&output);
    assert!(output.status.success(), "{shown}");
    let received = server.received();
    let body = &received[0].body;
    assert_eq!(body["model"], "m");
    assert_eq!(
        received[0].authorization.as_deref(),
        Some(&*format!("Bearer {FAKE_SECRET}"))
    );
    let offered: Vec<&str> = body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        offered,
        [
            "search",
            "glob",
            "read",
            "write",
            "edit",
            "bash",
            "command_output",
            "web_fetch",
            "agent"
        ],
        "no catalog, no gateway"
    );
}
