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
    /// Ranged read with line numbers.
    View {
        path: String,
        range: Option<(usize, usize)>,
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
            ToolCall::View { .. } => "view",
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
            ToolCall::View { path, .. }
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
        ("view", s.find("<view")),
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
        "view" => parse_view(s),
        "bash" => parse_bash(s),
        "grep" => parse_grep(s),
        "glob" => parse_glob(s),
        "run_tests" => Some(ToolCall::RunTests),
        "submit" => Some(ToolCall::Submit),
        _ => None,
    }
}

fn parse_view(s: &str) -> Option<ToolCall> {
    let rest = s.get(s.find("<view")?..)?;
    let path = extract_attr(rest, "path")?;
    let range = extract_attr(rest, "range").and_then(|r| parse_range(&r));
    Some(ToolCall::View { path, range })
}

fn parse_range(r: &str) -> Option<(usize, usize)> {
    let (a, b) = r.split_once('-')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
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
        ToolCall::View { path, range } => match rt.read_file(path) {
            Ok(content) => {
                viewed.insert(path.clone());
                Ok(render_view(&content, *range))
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
                return Ok(format!("ERROR: view {path} before editing it"));
            }
            let content = match rt.read_file(path) {
                Ok(c) => c,
                Err(e) => return Ok(format!("ERROR: cannot read {path}: {e}")),
            };
            match content.matches(old.as_str()).count() {
                0 => Ok("ERROR: <search> text not found exactly; re-view and copy it precisely".to_string()),
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

fn render_view(content: &str, range: Option<(usize, usize)>) -> String {
    let total = content.lines().count();
    let (start, end) = range.unwrap_or((1, total));
    let start = start.max(1);
    let mut out = String::new();
    for (idx, line) in content.lines().enumerate() {
        let n = idx + 1;
        if n >= start && n <= end {
            out.push_str(&format!("{n}\t{line}\n"));
        }
    }
    if out.is_empty() {
        out.push_str("(empty range or file)\n");
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
