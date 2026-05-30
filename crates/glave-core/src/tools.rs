//! The action grammar — the typed tool surface the model drives, modeled on
//! Anthropic's text-editor + bash + grep/glob tools. Model output is parsed into
//! a typed `ToolCall`, mediated by a `Policy`, then executed against the
//! `WorkspaceRuntime` port. Read-before-edit and exact-unique-match are enforced.

use anyhow::Result;
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
