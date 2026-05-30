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
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

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

/// Read `.glave/mcp.json` from the workspace. Missing/invalid -> no servers.
pub fn load_config(rt: &dyn WorkspaceRuntime) -> Vec<McpServerConfig> {
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
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            Some(McpServerConfig { name: name.clone(), command, args })
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
        Self { server: server.to_string(), transport }
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
        self.transport.notify("notifications/initialized", json!({}))
    }

    pub fn list_tools(&mut self) -> Result<Vec<McpTool>> {
        let res = self.transport.request("tools/list", json!({}))?;
        let tools = res.get("tools").and_then(Value::as_array).cloned().unwrap_or_default();
        Ok(tools
            .iter()
            .filter_map(|t| {
                let name = t.get("name").and_then(Value::as_str)?.to_string();
                let description =
                    t.get("description").and_then(Value::as_str).unwrap_or("").to_string();
                Some(McpTool { server: self.server.clone(), name, description })
            })
            .collect())
    }

    /// Call a tool; flatten the `content` text blocks into a single string.
    pub fn call_tool(&mut self, name: &str, args: &Value) -> Result<String> {
        let res =
            self.transport.request("tools/call", json!({"name": name, "arguments": args.clone()}))?;
        let blocks = res.get("content").and_then(Value::as_array).cloned().unwrap_or_default();
        let text = blocks
            .iter()
            .filter_map(|c| c.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(if text.is_empty() { res.to_string() } else { text })
    }
}

/// Real transport: newline-delimited JSON-RPC over a child process's stdio.
pub struct StdioTransport {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl StdioTransport {
    pub fn spawn(cfg: &McpServerConfig) -> Result<Self> {
        let mut child = Command::new(&cfg.command)
            .args(&cfg.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("spawning MCP server {}", cfg.name))?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("MCP server has no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("MCP server has no stdout"))?;
        Ok(Self { child, stdin, stdout: BufReader::new(stdout), next_id: 1 })
    }

    fn send(&mut self, msg: &Value) -> Result<()> {
        let line = serde_json::to_string(msg)?;
        self.stdin.write_all(line.as_bytes())?;
        self.stdin.write_all(b"\n")?;
        self.stdin.flush()?;
        Ok(())
    }

    /// Read lines until the response with `id` arrives; skip notifications and
    /// mismatched ids.
    fn read_response(&mut self, id: u64) -> Result<Value> {
        loop {
            let mut line = String::new();
            if self.stdout.read_line(&mut line)? == 0 {
                bail!("MCP server closed the connection");
            }
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
                responses: responses.into_iter().map(|(m, v)| (m.to_string(), v)).collect(),
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
        assert_eq!(cfg.get(1).map(|c| (c.name.as_str(), c.command.as_str())), Some(("zed", "z")));
        assert_eq!(cfg.get(1).map(|c| c.args.clone()), Some(vec!["--x".to_string()]));
    }

    #[test]
    fn parse_config_tolerates_garbage() {
        assert!(parse_config("not json").is_empty());
        assert!(parse_config("{}").is_empty());
        assert!(parse_config(r#"{"servers":{"bad":{"args":[]}}}"#).is_empty()); // no command
    }

    #[test]
    fn initialize_sends_request_then_notification() -> Result<()> {
        let t = FakeTransport::new(vec![("initialize", json!({"protocolVersion": "2024-11-05"}))]);
        let mut client = McpClient::new("srv", t);
        client.initialize()?;
        assert_eq!(client.transport.methods, ["initialize", "notify:notifications/initialized"]);
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
        assert_eq!(tools.first().map(|t| (t.name.as_str(), t.description.as_str(), t.server.as_str())),
                   Some(("search", "search the web", "srv")));
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
        assert_eq!(client.call_tool("search", &json!({"q": "x"}))?, "line1\nline2");
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
}
