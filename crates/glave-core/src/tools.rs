//! The action grammar — the typed tool surface the model drives, modeled on
//! Anthropic's text-editor + bash + grep/glob tools. Model output is parsed into
//! a typed `ToolCall`, mediated by a `Policy`, then executed against the
//! `WorkspaceRuntime` port. Read-before-edit and exact-unique-match are enforced.

use anyhow::{bail, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;

use crate::oracle;
use crate::ports::WorkspaceRuntime;
use crate::task::TaskSpec;

#[derive(Debug, Clone)]
pub enum ToolCall {
    /// Paginated read with line numbers, bounded by default (offset = first
    /// line, 1-based; limit = max lines). A file under the limit comes whole.
    Read {
        path: String,
        offset: Option<usize>,
        limit: Option<usize>,
    },
    /// Replace an exact, unique snippet (read-before-edit enforced).
    StrReplace {
        path: String,
        old: String,
        new: String,
    },
    /// Create or overwrite a whole file.
    Create {
        path: String,
        content: String,
    },
    /// Run a shell command in the sandbox (covers delete, build, …).
    Bash {
        cmd: String,
    },
    /// Search file contents (ripgrep, git-aware).
    Grep {
        pattern: String,
        glob: Option<String>,
    },
    /// Find files by glob (ripgrep, git-aware).
    Glob {
        pattern: String,
    },
    /// Run the task's oracle and report pass/fail.
    RunTests,
    /// Spawn a research subagent on a focused question; its summary returns as
    /// the observation. A context-compression worker — the main agent's context
    /// never sees the exploration, only the answer. `name` selects a configured
    /// agent type (`.glave/agents/*.md`); `None` is the default read-only researcher.
    Subagent {
        name: Option<String>,
        task: String,
    },
    /// Call a tool on a configured MCP server with JSON arguments.
    Mcp {
        server: String,
        tool: String,
        args: String,
    },
    /// Declare completion.
    Submit,
}

impl ToolCall {
    pub fn name(&self) -> &'static str {
        match self {
            ToolCall::Read { .. } => "read",
            ToolCall::StrReplace { .. } => "str_replace",
            ToolCall::Create { .. } => "create",
            ToolCall::Bash { .. } => "bash",
            ToolCall::Grep { .. } => "grep",
            ToolCall::Glob { .. } => "glob",
            ToolCall::RunTests => "run_tests",
            ToolCall::Subagent { .. } => "subagent",
            ToolCall::Mcp { .. } => "mcp",
            ToolCall::Submit => "submit",
        }
    }

    pub fn path(&self) -> Option<String> {
        match self {
            ToolCall::Read { path, .. }
            | ToolCall::StrReplace { path, .. }
            | ToolCall::Create { path, .. } => Some(path.clone()),
            _ => None,
        }
    }

    /// Build a typed `ToolCall` from a native tool call — the tool `name` and its
    /// raw JSON `arguments`. Arguments deserialize into a per-tool struct with
    /// `deny_unknown_fields`, so anything off-schema (wrong type, missing field,
    /// extra key) is rejected *here* rather than executed — the acceptance
    /// guarantee. Empty arguments are treated as `{}` (for no-arg tools).
    pub fn from_call(name: &str, arguments: &str) -> Result<ToolCall> {
        let trimmed = arguments.trim();
        let args = if trimmed.is_empty() { "{}" } else { trimmed };
        let call = match name {
            "read" => {
                let a: ReadArgs = serde_json::from_str(args)?;
                ToolCall::Read { path: a.path, offset: a.offset, limit: a.limit }
            }
            "str_replace" => {
                let a: StrReplaceArgs = serde_json::from_str(args)?;
                ToolCall::StrReplace { path: a.path, old: a.old, new: a.new }
            }
            "create" => {
                let a: CreateArgs = serde_json::from_str(args)?;
                ToolCall::Create { path: a.path, content: a.content }
            }
            "bash" => {
                let a: BashArgs = serde_json::from_str(args)?;
                ToolCall::Bash { cmd: a.cmd }
            }
            "grep" => {
                let a: GrepArgs = serde_json::from_str(args)?;
                ToolCall::Grep { pattern: a.pattern, glob: a.glob }
            }
            "glob" => {
                let a: GlobArgs = serde_json::from_str(args)?;
                ToolCall::Glob { pattern: a.pattern }
            }
            "run_tests" => ToolCall::RunTests,
            "subagent" => {
                let a: SubagentArgs = serde_json::from_str(args)?;
                ToolCall::Subagent { name: a.name, task: a.task }
            }
            "mcp" => {
                let a: McpArgs = serde_json::from_str(args)?;
                let mcp_args = if a.args.is_null() {
                    "{}".to_string()
                } else {
                    a.args.to_string()
                };
                ToolCall::Mcp { server: a.server, tool: a.tool, args: mcp_args }
            }
            "submit" => ToolCall::Submit,
            other => bail!("unknown tool: {other}"),
        };
        Ok(call)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadArgs {
    path: String,
    #[serde(default)]
    offset: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrReplaceArgs {
    path: String,
    old: String,
    new: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateArgs {
    path: String,
    content: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BashArgs {
    cmd: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GrepArgs {
    pattern: String,
    #[serde(default)]
    glob: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GlobArgs {
    pattern: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubagentArgs {
    #[serde(default)]
    name: Option<String>,
    task: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct McpArgs {
    server: String,
    tool: String,
    #[serde(default)]
    args: Value,
}

/// A tool declaration sent to the model: name + description + a JSON-Schema for
/// its arguments. The provider renders these into its native `tools` array, and
/// `from_call` validates incoming arguments against the same shape.
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
}

/// The built-in tool catalog declared to the model for native tool-calling.
#[must_use]
#[allow(clippy::too_many_lines)] // a flat schema-declaration list; splitting it adds no clarity
pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "read",
            description: "Read a file with line numbers. Bounded by default; pass offset/limit to page.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Workspace-relative file path"},
                    "offset": {"type": "integer", "description": "1-based first line"},
                    "limit": {"type": "integer", "description": "Max lines to return"}
                },
                "required": ["path"]
            }),
        },
        ToolSpec {
            name: "str_replace",
            description: "Replace an exact, unique snippet in a file. Read the file first.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "old": {"type": "string", "description": "Exact existing text (must be unique)"},
                    "new": {"type": "string", "description": "Replacement text"}
                },
                "required": ["path", "old", "new"]
            }),
        },
        ToolSpec {
            name: "create",
            description: "Create or overwrite a whole file.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "content": {"type": "string"}
                },
                "required": ["path", "content"]
            }),
        },
        ToolSpec {
            name: "bash",
            description: "Run a shell command in the sandbox (build, test, delete, …).",
            parameters: json!({
                "type": "object",
                "properties": {"cmd": {"type": "string"}},
                "required": ["cmd"]
            }),
        },
        ToolSpec {
            name: "grep",
            description: "Search file contents (ripgrep, git-aware).",
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string"},
                    "glob": {"type": "string", "description": "Optional file glob filter"}
                },
                "required": ["pattern"]
            }),
        },
        ToolSpec {
            name: "glob",
            description: "Find files by glob (ripgrep, git-aware).",
            parameters: json!({
                "type": "object",
                "properties": {"pattern": {"type": "string"}},
                "required": ["pattern"]
            }),
        },
        ToolSpec {
            name: "run_tests",
            description: "Run the task's oracle and report pass/fail.",
            parameters: json!({"type": "object", "properties": {}}),
        },
        ToolSpec {
            name: "subagent",
            description: "Spawn a focused research subagent; only its summary returns to you.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "Optional configured agent type"},
                    "task": {"type": "string", "description": "The focused question or task"}
                },
                "required": ["task"]
            }),
        },
        ToolSpec {
            name: "mcp",
            description: "Call a tool on a configured MCP server.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "server": {"type": "string"},
                    "tool": {"type": "string"},
                    "args": {"type": "object", "description": "Arguments for the MCP tool"}
                },
                "required": ["server", "tool"]
            }),
        },
        ToolSpec {
            name: "submit",
            description: "Declare the task complete.",
            parameters: json!({"type": "object", "properties": {}}),
        },
    ]
}

/// Parse the earliest recognized tool tag from model output.
pub fn parse(s: &str) -> Option<ToolCall> {
    let kind = [
        ("str_replace", s.find("<str_replace")),
        ("create", s.find("<create")),
        ("read", s.find("<read")),
        ("bash", s.find("<bash")),
        ("grep", s.find("<grep")),
        ("glob", s.find("<glob")),
        ("run_tests", s.find("<run_tests")),
        ("subagent", s.find("<subagent")),
        ("mcp", s.find("<mcp")),
        ("submit", s.find("<submit")),
    ]
    .into_iter()
    .filter_map(|(k, i)| i.map(|i| (k, i)))
    .min_by_key(|(_, i)| *i)?
    .0;

    match kind {
        "str_replace" => parse_str_replace(s),
        "create" => parse_create(s),
        "read" => parse_read(s),
        "bash" => parse_bash(s),
        "grep" => parse_grep(s),
        "glob" => parse_glob(s),
        "run_tests" => Some(ToolCall::RunTests),
        "subagent" => parse_subagent(s),
        "mcp" => parse_mcp(s),
        "submit" => Some(ToolCall::Submit),
        _ => None,
    }
}

/// Parse all `<create>` blocks in a response (the one-shot agent may rewrite
/// several files in a single reply).
pub fn parse_creates(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find("<create") {
        let Some(chunk) = rest.get(start..) else {
            break;
        };
        if let Some(ToolCall::Create { path, content }) = parse_create(chunk) {
            out.push((path, content));
        }
        let advance = chunk
            .find("</create>")
            .map_or(chunk.len(), |c| c + "</create>".len());
        let Some(next) = chunk.get(advance..) else {
            break;
        };
        rest = next;
    }
    out
}

fn parse_read(s: &str) -> Option<ToolCall> {
    let rest = s.get(s.find("<read")?..)?;
    let path = extract_attr(rest, "path")?;
    let offset = extract_attr(rest, "offset").and_then(|v| v.trim().parse().ok());
    let limit = extract_attr(rest, "limit").and_then(|v| v.trim().parse().ok());
    Some(ToolCall::Read { path, offset, limit })
}

fn parse_create(s: &str) -> Option<ToolCall> {
    let rest = s.get(s.find("<create")?..)?;
    let path = extract_attr(rest, "path")?;
    let open = rest.find('>')?.checked_add(1)?;
    let close = rest.find("</create>")?;
    if close < open {
        return None;
    }
    Some(ToolCall::Create {
        path,
        content: unwrap_block(rest.get(open..close)?),
    })
}

fn parse_mcp(s: &str) -> Option<ToolCall> {
    let rest = s.get(s.find("<mcp")?..)?;
    let server = extract_attr(rest, "server")?;
    let tool = extract_attr(rest, "tool")?;
    let open = rest.find('>')?.checked_add(1)?;
    let close = rest.find("</mcp>")?;
    if close < open {
        return None;
    }
    let args = rest.get(open..close)?.trim().to_string();
    Some(ToolCall::Mcp { server, tool, args })
}

fn parse_subagent(s: &str) -> Option<ToolCall> {
    let rest = s.get(s.find("<subagent")?..)?;
    let name = extract_attr(rest, "name");
    let open = rest.find('>')?.checked_add(1)?;
    let close = rest.find("</subagent>")?;
    if close < open {
        return None;
    }
    let task = rest.get(open..close)?.trim().to_string();
    if task.is_empty() {
        return None;
    }
    Some(ToolCall::Subagent { name, task })
}

fn parse_bash(s: &str) -> Option<ToolCall> {
    let rest = s.get(s.find("<bash")?..)?;
    let open = rest.find('>')?.checked_add(1)?;
    let close = rest.find("</bash>")?;
    if close < open {
        return None;
    }
    let cmd = rest.get(open..close)?.trim().to_string();
    if cmd.is_empty() {
        return None;
    }
    Some(ToolCall::Bash { cmd })
}

fn parse_grep(s: &str) -> Option<ToolCall> {
    let rest = s.get(s.find("<grep")?..)?;
    let pattern = extract_attr(rest, "pattern")?;
    let glob = extract_attr(rest, "glob");
    Some(ToolCall::Grep { pattern, glob })
}

fn parse_glob(s: &str) -> Option<ToolCall> {
    let rest = s.get(s.find("<glob")?..)?;
    let pattern = extract_attr(rest, "pattern")?;
    Some(ToolCall::Glob { pattern })
}

fn parse_str_replace(s: &str) -> Option<ToolCall> {
    let rest = s.get(s.find("<str_replace")?..)?;
    let path = extract_attr(rest, "path")?;
    let old = unwrap_block(between(rest, "<search>", "</search>")?);
    let new = unwrap_block(between(rest, "<replace>", "</replace>")?);
    Some(ToolCall::StrReplace { path, old, new })
}

fn between<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let i = s.find(open)?.checked_add(open.len())?;
    let after = s.get(i..)?;
    let j = after.find(close)?;
    after.get(..j)
}

fn extract_attr(s: &str, attr: &str) -> Option<String> {
    let key = format!("{attr}=\"");
    let start = s.find(&key)?.checked_add(key.len())?;
    let rest = s.get(start..)?;
    let end = rest.find('"')?;
    Some(rest.get(..end)?.to_string())
}

fn unwrap_block(s: &str) -> String {
    let s = s.strip_prefix('\n').unwrap_or(s);
    let s = s.strip_suffix('\n').unwrap_or(s);
    s.to_string()
}

/// Execute an allowed tool call against the runtime, returning the observation
/// fed back to the model.
pub fn execute(
    call: &ToolCall,
    rt: &mut dyn WorkspaceRuntime,
    task: &TaskSpec,
    viewed: &mut BTreeSet<String>,
) -> Result<String> {
    match call {
        ToolCall::Read { path, offset, limit } => match rt.read_file(path) {
            Ok(content) => {
                viewed.insert(path.clone());
                Ok(render_read(&content, *offset, *limit))
            }
            Err(e) => Ok(format!("ERROR: cannot read {path}: {e}")),
        },
        ToolCall::Create { path, content } => {
            rt.write_file(path, content)?;
            viewed.insert(path.clone());
            Ok(format!("created {path} ({} bytes)", content.len()))
        }
        ToolCall::StrReplace { path, old, new } => {
            if !viewed.contains(path) {
                return Ok(format!("ERROR: read {path} before editing it"));
            }
            let content = match rt.read_file(path) {
                Ok(c) => c,
                Err(e) => return Ok(format!("ERROR: cannot read {path}: {e}")),
            };
            match content.matches(old.as_str()).count() {
                0 => Ok("ERROR: <search> text not found exactly; re-read and copy it precisely".to_string()),
                1 => {
                    let updated = content.replacen(old.as_str(), new, 1);
                    rt.write_file(path, &updated)?;
                    Ok(format!("edited {path}"))
                }
                n => Ok(format!(
                    "ERROR: <search> text matches {n} places; add surrounding context to make it unique"
                )),
            }
        }
        ToolCall::Bash { cmd } => {
            let r = rt.run(cmd)?;
            Ok(format!(
                "$ {cmd}\n[exit {}]\n{}",
                r.exit_code,
                clip(&merge(&r.stdout, &r.stderr), 4000)
            ))
        }
        ToolCall::Grep { pattern, glob } => {
            let mut cmd = String::from("rg --line-number --no-heading --color never");
            if let Some(g) = glob {
                cmd.push_str(&format!(" -g {}", sq(g)));
            }
            cmd.push_str(&format!(" -e {}", sq(pattern)));
            let r = rt.run(&cmd)?;
            let out = clip(&merge(&r.stdout, &r.stderr), 4000);
            Ok(if out.trim().is_empty() {
                "no matches".to_string()
            } else {
                out
            })
        }
        ToolCall::Glob { pattern } => {
            let r = rt.run(&format!("rg --files -g {}", sq(pattern)))?;
            let out = clip(&merge(&r.stdout, &r.stderr), 4000);
            Ok(if out.trim().is_empty() {
                "no files match".to_string()
            } else {
                out
            })
        }
        ToolCall::RunTests => {
            let v = oracle::evaluate(&*rt, task)?;
            Ok(format!(
                "TEST RESULTS\nFAIL_TO_PASS: {}\nPASS_TO_PASS: {}\n---\n{}",
                if v.fail_to_pass_ok { "PASS" } else { "FAIL" },
                if v.pass_to_pass_ok { "PASS" } else { "FAIL" },
                clip(&v.f2p_output, 2000)
            ))
        }
        // Handled in the agent loop (need provider / MCP hub); never reached here.
        ToolCall::Subagent { .. } => {
            Ok("ERROR: subagent is dispatched by the agent loop".to_string())
        }
        ToolCall::Mcp { .. } => Ok("ERROR: mcp is dispatched by the agent loop".to_string()),
        ToolCall::Submit => Ok("submitted".to_string()),
    }
}

/// Default read window when the model gives no `limit` — bounds context so a
/// large file can't flood it; a file under this size still comes back whole.
const READ_DEFAULT_LIMIT: usize = 250;

fn render_read(content: &str, offset: Option<usize>, limit: Option<usize>) -> String {
    let total = content.lines().count();
    let start = offset.unwrap_or(1).max(1);
    let lim = limit.unwrap_or(READ_DEFAULT_LIMIT).max(1);
    let end = start.saturating_add(lim).saturating_sub(1).min(total);
    let mut out = String::new();
    for (idx, line) in content.lines().enumerate() {
        let n = idx + 1;
        if n >= start && n <= end {
            out.push_str(&format!("{n}\t{line}\n"));
        }
    }
    if out.is_empty() {
        return format!("(file has {total} lines; offset {start} is past the end)\n");
    }
    if start > 1 || end < total {
        out.push_str(&format!(
            "[lines {start}-{end} of {total}; read offset={} to continue, or grep to jump]\n",
            end.saturating_add(1)
        ));
    }
    out
}

fn merge(stdout: &str, stderr: &str) -> String {
    format!("{}\n{}", stdout.trim_end(), stderr.trim_end())
        .trim()
        .to_string()
}

fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Single-quote a string for safe inclusion in a `sh -c` command.
fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_read_with_offset_and_limit() {
        let call = parse("<read path=\"a.rs\" offset=\"10\" limit=\"40\"/>");
        assert!(matches!(call, Some(ToolCall::Read { offset: Some(10), limit: Some(40), .. })));
    }

    #[test]
    fn parse_read_defaults_to_none() {
        let call = parse("<read path=\"a.rs\"/>");
        assert!(matches!(call, Some(ToolCall::Read { offset: None, limit: None, .. })));
    }

    #[test]
    fn parse_subagent_extracts_task() {
        let call = parse("<subagent>find the bug</subagent>");
        assert!(matches!(&call, Some(ToolCall::Subagent { task, .. }) if task == "find the bug"));
    }

    #[test]
    fn parse_mcp_extracts_server_tool_args() {
        let call = parse("<mcp server=\"web\" tool=\"search\">{\"q\":\"x\"}</mcp>");
        assert!(matches!(
            &call,
            Some(ToolCall::Mcp { server, tool, args })
                if server == "web" && tool == "search" && args == "{\"q\":\"x\"}"
        ));
    }

    #[test]
    fn parse_picks_earliest_tag() {
        let call = parse("noise <read path=\"a\"/> then <submit/>");
        assert_eq!(call.map(|c| c.name()), Some("read"));
    }

    #[test]
    fn str_replace_requires_path() {
        assert!(parse("<str_replace><search>\na\n</search><replace>\nb\n</replace></str_replace>").is_none());
    }

    // --- native tool-calling: from_call + tool_specs ---

    #[test]
    fn from_call_parses_typed_args() -> Result<()> {
        let c = ToolCall::from_call("read", r#"{"path":"a.rs","offset":10,"limit":40}"#)?;
        assert!(matches!(c, ToolCall::Read { offset: Some(10), limit: Some(40), .. }));
        let c = ToolCall::from_call("bash", r#"{"cmd":"cargo test"}"#)?;
        assert!(matches!(&c, ToolCall::Bash { cmd } if cmd == "cargo test"));
        Ok(())
    }

    #[test]
    fn from_call_rejects_unknown_fields() {
        // deny_unknown_fields: anything off-schema is rejected, not executed.
        assert!(ToolCall::from_call("bash", r#"{"cmd":"ls","extra":"x"}"#).is_err());
    }

    #[test]
    fn from_call_rejects_wrong_type_and_missing_field() {
        assert!(ToolCall::from_call("read", r#"{"path":42}"#).is_err());
        assert!(ToolCall::from_call("str_replace", r#"{"path":"a"}"#).is_err());
    }

    #[test]
    fn from_call_handles_no_arg_tools() -> Result<()> {
        assert!(matches!(ToolCall::from_call("run_tests", "")?, ToolCall::RunTests));
        assert!(matches!(ToolCall::from_call("submit", "{}")?, ToolCall::Submit));
        Ok(())
    }

    #[test]
    fn from_call_rejects_unknown_tool() {
        assert!(ToolCall::from_call("frobnicate", "{}").is_err());
    }

    #[test]
    fn tool_specs_cover_every_variant() {
        let names: Vec<&str> = tool_specs().iter().map(|s| s.name).collect();
        for n in [
            "read", "str_replace", "create", "bash", "grep", "glob", "run_tests", "subagent",
            "mcp", "submit",
        ] {
            assert!(names.contains(&n), "missing spec: {n}");
        }
    }
}
