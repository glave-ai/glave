//! MCP (Model Context Protocol) client — connects to operator-configured tool
//! servers and exposes their tools to the agent.
//!
//! Trust model (see `context/security.md`): servers listed in `.glave/mcp.json`
//! are **operator-configured and therefore trusted**, run brain-side with normal
//! access — the same model as Claude Code's `.mcp.json`. Arbitrary/untrusted MCP
//! servers (actuator-sandboxed) are a deliberate future extension.
//!
//! The protocol logic lives in [`McpClient`] over an [`McpTransport`] port, so it
//! is unit-tested with a fake transport; [`StdioTransport`] is the thin real-IO
//! adapter (newline-delimited JSON-RPC over a child process's stdio).

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::ports::WorkspaceRuntime;

/// One configured MCP server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
}

/// A tool advertised by a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpTool {
    pub server: String,
    pub name: String,
    pub description: String,
}

/// Read MCP server config from the workspace's `.glave/mcp.json`.
///
/// SECURITY: the workspace is untrusted — it can be an ingested or attacker-
/// controlled repo. Spawning processes from a workspace-provided config is
/// remote code execution on the trusted brain, so it is **off by default**;
/// the operator must explicitly opt into trusting this workspace via
/// `GLAVE_TRUST_WORKSPACE_MCP`. (Operator-sourced config — the proper model —
/// arrives with the config/gateway work and bypasses this entirely.)
pub fn load_config(rt: &dyn WorkspaceRuntime) -> Vec<McpServerConfig> {
    if std::env::var("GLAVE_TRUST_WORKSPACE_MCP").is_err() {
        return Vec::new();
    }
    rt.read_file(".glave/mcp.json")
        .map(|raw| parse_config(&raw))
        .unwrap_or_default()
}

/// Parse `{ "servers": { "<name>": { "command": "...", "args": ["..."] } } }`.
/// Tolerant: anything malformed yields an empty list, sorted by name.
pub fn parse_config(raw: &str) -> Vec<McpServerConfig> {
    let Ok(v) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };
    let Some(servers) = v.get("servers").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut out: Vec<McpServerConfig> = servers
        .iter()
        .filter_map(|(name, cfg)| {
            let command = cfg.get("command").and_then(Value::as_str)?.to_string();
            let args = cfg
                .get("args")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            Some(McpServerConfig {
                name: name.clone(),
                command,
                args,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Synchronous JSON-RPC transport to one MCP server.
pub trait McpTransport {
    fn request(&mut self, method: &str, params: Value) -> Result<Value>;
    fn notify(&mut self, method: &str, params: Value) -> Result<()>;
}

/// Speaks MCP over a transport: initialize handshake, list tools, call a tool.
pub struct McpClient<T: McpTransport> {
    server: String,
    transport: T,
}

impl<T: McpTransport> McpClient<T> {
    pub fn new(server: &str, transport: T) -> Self {
        Self {
            server: server.to_string(),
            transport,
        }
    }

    /// `initialize` request + `initialized` notification.
    pub fn initialize(&mut self) -> Result<()> {
        self.transport.request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "glave", "version": "0"}
            }),
        )?;
        self.transport
            .notify("notifications/initialized", json!({}))
    }

    pub fn list_tools(&mut self) -> Result<Vec<McpTool>> {
        let res = self.transport.request("tools/list", json!({}))?;
        let tools = res
            .get("tools")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(tools
            .iter()
            .filter_map(|t| {
                let name = t.get("name").and_then(Value::as_str)?.to_string();
                let description = t
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                Some(McpTool {
                    server: self.server.clone(),
                    name,
                    description,
                })
            })
            .collect())
    }

    /// Call a tool; flatten the `content` text blocks into a single string.
    pub fn call_tool(&mut self, name: &str, args: &Value) -> Result<String> {
        let res = self.transport.request(
            "tools/call",
            json!({"name": name, "arguments": args.clone()}),
        )?;
        let blocks = res
            .get("content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let text = blocks
            .iter()
            .filter_map(|c| c.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(if text.is_empty() {
            res.to_string()
        } else {
            text
        })
    }
}

/// Real transport: newline-delimited JSON-RPC over a child process's stdio.
fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

pub struct StdioTransport {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    next_id: u64,
    read_timeout: Duration,
}

impl StdioTransport {
    pub fn spawn(cfg: &McpServerConfig) -> Result<Self> {
        let mut child = Command::new(&cfg.command)
            .args(&cfg.args)
            // SECURITY: don't leak brain secrets (e.g. model API keys) into the
            // server's environment — pass only a minimal env so it still resolves.
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("spawning MCP server {}", cfg.name))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("MCP server has no stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("MCP server has no stdout"))?;

        // Read on a dedicated thread so a silent/hung server can't block the
        // agent: the request path waits on the channel with a deadline, not on a
        // raw blocking read. The thread exits when the child's stdout closes —
        // which Drop forces by killing the child.
        let (tx, rx) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break, // EOF or read error: close the channel
                    Ok(_) => {
                        if tx.send(line).is_err() {
                            break; // receiver gone (transport dropped)
                        }
                    }
                }
            }
        });

        Ok(Self {
            child,
            stdin,
            lines: rx,
            next_id: 1,
            read_timeout: Duration::from_secs(env_u64("GLAVE_MCP_TIMEOUT_SECS", 30)),
        })
    }

    fn send(&mut self, msg: &Value) -> Result<()> {
        let line = serde_json::to_string(msg)?;
        self.stdin.write_all(line.as_bytes())?;
        self.stdin.write_all(b"\n")?;
        self.stdin.flush()?;
        Ok(())
    }

    /// Wait for the response with `id`, skipping notifications and mismatched
    /// ids, up to the read deadline. A silent server fails the request rather
    /// than hanging the agent forever.
    fn read_response(&mut self, id: u64) -> Result<Value> {
        let deadline = Instant::now() + self.read_timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                bail!(
                    "MCP request timed out after {}s",
                    self.read_timeout.as_secs()
                );
            }
            let line = match self.lines.recv_timeout(remaining) {
                Ok(line) => line,
                Err(RecvTimeoutError::Timeout) => {
                    bail!(
                        "MCP request timed out after {}s",
                        self.read_timeout.as_secs()
                    )
                }
                Err(RecvTimeoutError::Disconnected) => {
                    bail!("MCP server closed the connection")
                }
            };
            let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
                continue;
            };
            if v.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(err) = v.get("error") {
                    bail!("MCP error: {err}");
                }
                return Ok(v.get("result").cloned().unwrap_or(Value::Null));
            }
        }
    }
}

impl McpTransport for StdioTransport {
    fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        self.read_response(id)
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Connects every configured server and routes tool calls by server name. Owns
/// the live clients for a task's duration; dropping it kills the server processes.
pub struct McpHub {
    clients: Vec<(String, McpClient<StdioTransport>)>,
    tools: Vec<McpTool>,
}

impl McpHub {
    /// Connect every server in `.glave/mcp.json`. A server that fails to spawn or
    /// handshake is skipped (logged), never fatal — MCP is opt-in.
    /// Connect to whatever servers the (gated) workspace config names. Secure by
    /// default: an untrusted workspace yields no servers (see [`load_config`]).
    #[must_use]
    pub fn connect(rt: &dyn WorkspaceRuntime) -> Self {
        Self::from_configs(&load_config(rt))
    }

    /// Connect to an explicit, operator-trusted set of servers.
    #[must_use]
    pub fn from_configs(configs: &[McpServerConfig]) -> Self {
        let mut clients = Vec::new();
        let mut tools = Vec::new();
        for cfg in configs {
            match Self::connect_one(cfg) {
                Ok((client, mut ts)) => {
                    tools.append(&mut ts);
                    clients.push((cfg.name.clone(), client));
                }
                Err(e) => eprintln!("MCP server {} unavailable: {e}", cfg.name),
            }
        }
        Self { clients, tools }
    }

    fn connect_one(cfg: &McpServerConfig) -> Result<(McpClient<StdioTransport>, Vec<McpTool>)> {
        let transport = StdioTransport::spawn(cfg)?;
        let mut client = McpClient::new(&cfg.name, transport);
        client.initialize()?;
        let tools = client.list_tools()?;
        Ok((client, tools))
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }

    #[must_use]
    pub fn tools(&self) -> &[McpTool] {
        &self.tools
    }

    /// Prompt catalog of available tools + the call syntax.
    #[must_use]
    pub fn catalog(&self) -> String {
        let mut out = String::from(
            "MCP TOOLS available — call with <mcp server=\"S\" tool=\"T\">{json args}</mcp>:\n",
        );
        for t in &self.tools {
            out.push_str(&format!("- {}/{} — {}\n", t.server, t.name, t.description));
        }
        out
    }

    /// Route a tool call to the named server.
    pub fn call(&mut self, server: &str, tool: &str, args: &Value) -> Result<String> {
        let client = self
            .clients
            .iter_mut()
            .find(|(n, _)| n == server)
            .map(|(_, c)| c)
            .ok_or_else(|| anyhow!("no MCP server named '{server}'"))?;
        client.call_tool(tool, args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct FakeTransport {
        responses: HashMap<String, Value>,
        methods: Vec<String>,
    }

    impl FakeTransport {
        fn new(responses: Vec<(&str, Value)>) -> Self {
            Self {
                responses: responses
                    .into_iter()
                    .map(|(m, v)| (m.to_string(), v))
                    .collect(),
                methods: Vec::new(),
            }
        }
    }

    impl McpTransport for FakeTransport {
        fn request(&mut self, method: &str, _params: Value) -> Result<Value> {
            self.methods.push(method.to_string());
            self.responses
                .get(method)
                .cloned()
                .ok_or_else(|| anyhow!("no scripted response for {method}"))
        }
        fn notify(&mut self, method: &str, _params: Value) -> Result<()> {
            self.methods.push(format!("notify:{method}"));
            Ok(())
        }
    }

    #[test]
    fn parse_config_reads_servers_sorted() {
        let cfg = parse_config(
            r#"{"servers":{"zed":{"command":"z","args":["--x"]},"abc":{"command":"a"}}}"#,
        );
        assert_eq!(cfg.len(), 2);
        assert_eq!(cfg.first().map(|c| c.name.as_str()), Some("abc"));
        assert_eq!(
            cfg.get(1).map(|c| (c.name.as_str(), c.command.as_str())),
            Some(("zed", "z"))
        );
        assert_eq!(
            cfg.get(1).map(|c| c.args.clone()),
            Some(vec!["--x".to_string()])
        );
    }

    #[test]
    fn parse_config_tolerates_garbage() {
        assert!(parse_config("not json").is_empty());
        assert!(parse_config("{}").is_empty());
        assert!(parse_config(r#"{"servers":{"bad":{"args":[]}}}"#).is_empty()); // no command
    }

    #[test]
    fn initialize_sends_request_then_notification() -> Result<()> {
        let t = FakeTransport::new(vec![(
            "initialize",
            json!({"protocolVersion": "2024-11-05"}),
        )]);
        let mut client = McpClient::new("srv", t);
        client.initialize()?;
        assert_eq!(
            client.transport.methods,
            ["initialize", "notify:notifications/initialized"]
        );
        Ok(())
    }

    #[test]
    fn list_tools_parses_names_and_descriptions() -> Result<()> {
        let t = FakeTransport::new(vec![(
            "tools/list",
            json!({"tools": [{"name": "search", "description": "search the web"}, {"name": "fetch"}]}),
        )]);
        let mut client = McpClient::new("srv", t);
        let tools = client.list_tools()?;
        assert_eq!(tools.len(), 2);
        assert_eq!(
            tools
                .first()
                .map(|t| (t.name.as_str(), t.description.as_str(), t.server.as_str())),
            Some(("search", "search the web", "srv"))
        );
        assert_eq!(tools.get(1).map(|t| t.description.as_str()), Some("")); // missing desc -> empty
        Ok(())
    }

    #[test]
    fn call_tool_flattens_text_content() -> Result<()> {
        let t = FakeTransport::new(vec![(
            "tools/call",
            json!({"content": [{"type": "text", "text": "line1"}, {"type": "text", "text": "line2"}]}),
        )]);
        let mut client = McpClient::new("srv", t);
        assert_eq!(
            client.call_tool("search", &json!({"q": "x"}))?,
            "line1\nline2"
        );
        Ok(())
    }

    #[test]
    fn stdio_transport_talks_to_a_real_server() -> Result<()> {
        // A tiny real MCP server (newline-delimited JSON-RPC) proves the stdio
        // adapter end-to-end, not just the client logic.
        let script = "import sys, json\n\
for line in sys.stdin:\n\
\x20   line = line.strip()\n\
\x20   if not line:\n\
\x20       continue\n\
\x20   msg = json.loads(line)\n\
\x20   mid = msg.get(\"id\")\n\
\x20   if mid is None:\n\
\x20       continue\n\
\x20   method = msg.get(\"method\")\n\
\x20   if method == \"initialize\":\n\
\x20       res = {\"protocolVersion\": \"2024-11-05\"}\n\
\x20   elif method == \"tools/list\":\n\
\x20       res = {\"tools\": [{\"name\": \"echo\", \"description\": \"echoes input\"}]}\n\
\x20   elif method == \"tools/call\":\n\
\x20       args = msg.get(\"params\", {}).get(\"arguments\", {})\n\
\x20       res = {\"content\": [{\"type\": \"text\", \"text\": \"echo:\" + json.dumps(args)}]}\n\
\x20   else:\n\
\x20       res = {}\n\
\x20   sys.stdout.write(json.dumps({\"jsonrpc\": \"2.0\", \"id\": mid, \"result\": res}) + \"\\n\")\n\
\x20   sys.stdout.flush()\n";
        let path = std::env::temp_dir().join(format!("glave_mcp_mock_{}.py", std::process::id()));
        std::fs::write(&path, script)?;
        let cfg = McpServerConfig {
            name: "mock".to_string(),
            command: "python3".to_string(),
            args: vec![path.to_string_lossy().into_owned()],
        };
        let Ok(transport) = StdioTransport::spawn(&cfg) else {
            eprintln!("skipping MCP integration test: python3 unavailable");
            let _ = std::fs::remove_file(&path);
            return Ok(());
        };
        let mut client = McpClient::new("mock", transport);
        client.initialize()?;
        let tools = client.list_tools()?;
        assert_eq!(tools.first().map(|t| t.name.as_str()), Some("echo"));
        let out = client.call_tool("echo", &json!({"hi": 1}))?;
        assert!(out.contains("echo:"), "real server echoed: {out}");
        let _ = std::fs::remove_file(&path);
        Ok(())
    }

    const MOCK_SERVER: &str = r#"import sys, json
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    msg = json.loads(line)
    mid = msg.get("id")
    if mid is None:
        continue
    method = msg.get("method")
    if method == "initialize":
        res = {"protocolVersion": "2024-11-05"}
    elif method == "tools/list":
        res = {"tools": [{"name": "echo", "description": "echoes input"}]}
    elif method == "tools/call":
        args = msg.get("params", {}).get("arguments", {})
        res = {"content": [{"type": "text", "text": "echo:" + json.dumps(args)}]}
    else:
        res = {}
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": mid, "result": res}) + "\n")
    sys.stdout.flush()
"#;

    #[test]
    fn hub_connects_lists_and_calls_from_configs() -> Result<()> {
        let path = std::env::temp_dir().join(format!("glave_mcp_hub_{}.py", std::process::id()));
        std::fs::write(&path, MOCK_SERVER)?;
        let cfg = McpServerConfig {
            name: "mock".to_string(),
            command: "python3".to_string(),
            args: vec![path.to_string_lossy().into_owned()],
        };

        let mut hub = McpHub::from_configs(std::slice::from_ref(&cfg));
        if hub.is_empty() {
            eprintln!("skipping MCP hub test: python3 unavailable");
            let _ = std::fs::remove_file(&path);
            return Ok(());
        }
        assert!(
            hub.catalog().contains("mock/echo"),
            "catalog: {}",
            hub.catalog()
        );
        let out = hub.call("mock", "echo", &json!({"x": 1}))?;
        assert!(out.contains("echo:"), "hub call: {out}");
        let _ = std::fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn load_config_ignores_untrusted_workspace_by_default() {
        use crate::testkit::InMemoryRuntime;
        // No GLAVE_TRUST_WORKSPACE_MCP is set in tests, so a workspace-provided
        // (potentially malicious) config must be ignored — no RCE by default.
        let rt = InMemoryRuntime::new()
            .with_file(".glave/mcp.json", r#"{"servers":{"x":{"command":"evil"}}}"#);
        assert!(load_config(&rt).is_empty());
    }
}
