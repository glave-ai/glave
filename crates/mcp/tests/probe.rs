//! A server slow to answer its first request is spoken to in the protocol it speaks. glave's
//! probe for MCP 2026-07-28 waits 10 s alone, then the older handshake is offered beside it,
//! and the first answer that says which protocol the server speaks decides: a server on the
//! newer one that answers late is still spoken to in it, and one on the older one that answers
//! the probe late, or never, in the older. On a paused clock, so the 10 s pass without waiting,
//! and the server answers only once the handshake has been offered: nothing depends on timing.

use glave_mcp::{Limits, McpServer};
use glave_ports::ToolSource as _;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, DuplexStream};

/// Which of the two first requests an answer is to.
#[derive(Clone, Copy)]
enum To {
    Probe,
    Handshake,
}

/// A server slow to answer, and what it says once the handshake has been offered.
struct Slow {
    /// What it is, for a failure's message.
    name: &'static str,
    /// Its answers, in order: to which request, and what.
    answers: Vec<(To, Value)>,
    /// Whether glave should speak MCP 2026-07-28 with it, rather than the older protocol.
    newer: bool,
}

/// A discovery: the server speaks MCP 2026-07-28.
fn discovered() -> Value {
    json!({"result": {
        "resultType": "complete",
        "supportedVersions": ["2026-07-28"],
        "capabilities": {"tools": {}},
        "ttlMs": 0,
        "cacheScope": "public",
        "_meta": {"io.modelcontextprotocol/serverInfo": {"name": "slow", "version": "1"}},
    }})
}

/// The older protocol's handshake, answered.
fn initialized() -> Value {
    json!({"result": {
        "protocolVersion": "2025-11-25",
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "slow", "version": "1"},
    }})
}

/// Serves glave at `theirs` as `slow` says, then a listing of one tool in the protocol glave
/// chose, which must be the one `slow` speaks.
async fn serve(theirs: DuplexStream, slow: &Slow) {
    let (read, mut write) = tokio::io::split(theirs);
    let mut lines = BufReader::new(read).lines();
    let mut next = async || -> Value {
        let line = lines.next_line().await.unwrap().unwrap();
        serde_json::from_str(&line).unwrap()
    };
    let mut answer = async |message: Value| {
        let line = format!("{message}\n");
        write.write_all(line.as_bytes()).await.unwrap();
    };
    let name = slow.name;
    let probe = next().await;
    assert_eq!(probe["method"], "server/discover", "{name}: {probe}");
    let meta = &probe["params"]["_meta"];
    assert_eq!(
        meta["io.modelcontextprotocol/protocolVersion"],
        "2026-07-28"
    );
    let initialize = next().await;
    assert_eq!(initialize["method"], "initialize", "{name}: {initialize}");
    assert_eq!(initialize["params"]["protocolVersion"], "2025-11-25");
    assert!(
        initialize["params"]["capabilities"]
            .get("elicitation")
            .is_none(),
        "the older protocol is not told it can ask: {initialize}"
    );
    for (to, said) in &slow.answers {
        let mut said = said.clone();
        said["jsonrpc"] = json!("2.0");
        said["id"] = match to {
            To::Probe => probe["id"].clone(),
            To::Handshake => initialize["id"].clone(),
        };
        answer(said).await;
    }
    if !slow.newer {
        let initialized = next().await;
        assert_eq!(
            initialized["method"], "notifications/initialized",
            "{name}: {initialized}"
        );
    }
    let listing = next().await;
    assert_eq!(listing["method"], "tools/list", "{name}: {listing}");
    let version = &listing["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"];
    assert_eq!(
        version.as_str() == Some("2026-07-28"),
        slow.newer,
        "{name}: spoken to in the protocol it speaks: {listing}"
    );
    answer(json!({"jsonrpc": "2.0", "id": listing["id"], "result": {
        "tools": [{"name": "echo", "inputSchema": {"type": "object"}}],
    }}))
    .await;
}

#[test]
fn a_server_slow_to_answer_is_spoken_to_in_the_protocol_it_speaks() {
    let refused = json!({"error": {"code": -32601, "message": "Method not found"}});
    let unsupported = json!({"error": {
        "code": -32022,
        "message": "Unsupported protocol version",
        "data": {"supported": ["2026-07-28"], "requested": "2025-11-25"},
    }});
    let servers = [
        Slow {
            name: "older, refusing the probe late",
            answers: vec![(To::Probe, refused), (To::Handshake, initialized())],
            newer: false,
        },
        Slow {
            name: "older, never answering the probe",
            answers: vec![(To::Handshake, initialized())],
            newer: false,
        },
        Slow {
            name: "both, answering the probe late",
            answers: vec![(To::Probe, discovered()), (To::Handshake, initialized())],
            newer: true,
        },
        Slow {
            name: "newer only, refusing the handshake first",
            answers: vec![(To::Handshake, unsupported), (To::Probe, discovered())],
            newer: true,
        },
    ];
    for slow in servers {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .unwrap();
        let name = slow.name;
        runtime.block_on(async {
            let (ours, theirs) = tokio::io::duplex(64 * 1024);
            let serving = tokio::spawn(async move { serve(theirs, &slow).await });
            let server = McpServer::connect("slow", ours, Limits::default());
            let tools = server.tools().await;
            let names: Vec<String> = tools
                .unwrap_or_else(|error| panic!("{name}: {error}"))
                .into_iter()
                .map(|tool| tool.spec.name)
                .collect();
            assert_eq!(names, ["mcp__slow__echo"], "{name}");
            serving.await.unwrap();
            server.close().await;
        });
    }
}
