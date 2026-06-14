//! Host-filesystem primitives shared by every on-host runtime — the local
//! client's [`crate::local::LocalRuntime`] and the Docker adapter's host-side
//! search/IO. Pure std plus the ripgrep/ignore crates; no container, no
//! network. Living here keeps the two runtimes byte-for-byte identical on
//! search, path safety, and command execution.

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use wait_timeout::ChildExt;

use crate::ports::ExecResult;

/// Dirs never worth searching/listing — vendoring and build output.
const SKIP: [&str; 6] = [
    "vendor",
    "target",
    "node_modules",
    ".git",
    ".cargo",
    ".venv",
];

/// `.gitignore`-aware walk that also prunes the [`SKIP`] dirs, honoring ignore
/// files even before a directory is a git checkout.
fn walk(root: &Path) -> ignore::Walk {
    ignore::WalkBuilder::new(root)
        .hidden(false) // show dotfiles; .gitignore + SKIP still prune
        .require_git(false)
        .filter_entry(|e| e.file_name().to_str().is_none_or(|n| !SKIP.contains(&n)))
        .build()
}

/// Git-aware content search via ripgrep's crates (no `rg` binary). Returns
/// `path:line:text`, optionally restricted to files matching `file_glob`.
pub fn grep(root: &Path, pattern: &str, file_glob: Option<&str>) -> Result<Vec<String>> {
    use globset::Glob;
    use grep::regex::RegexMatcher;
    use grep::searcher::sinks::UTF8;
    use grep::searcher::SearcherBuilder;

    let matcher = RegexMatcher::new(pattern).map_err(|e| anyhow!("invalid search pattern: {e}"))?;
    let glob = file_glob
        .map(|g| Glob::new(g).map(|g| g.compile_matcher()))
        .transpose()
        .map_err(|e| anyhow!("invalid file glob: {e}"))?;
    let mut searcher = SearcherBuilder::new().line_number(true).build();
    let mut out = Vec::new();
    for entry in walk(root) {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap_or(path);
        if let Some(g) = &glob {
            if !g.is_match(rel) {
                continue;
            }
        }
        let rel_disp = rel.display().to_string();
        // Per-file errors (binary / non-UTF8) are skipped, not fatal.
        let _ = searcher.search_path(
            &matcher,
            path,
            UTF8(|lnum, line| {
                out.push(format!("{rel_disp}:{lnum}:{}", line.trim_end()));
                Ok(true)
            }),
        );
    }
    Ok(out)
}

/// Git-aware file search by glob. Returns sorted workspace-relative paths.
pub fn find(root: &Path, glob: &str) -> Result<Vec<String>> {
    use globset::Glob;
    let matcher = Glob::new(glob)
        .map_err(|e| anyhow!("invalid glob: {e}"))?
        .compile_matcher();
    let mut out = Vec::new();
    for entry in walk(root) {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap_or(path);
        if matcher.is_match(rel) {
            out.push(rel.display().to_string());
        }
    }
    out.sort();
    Ok(out)
}

/// Git-aware listing of every file (sorted, workspace-relative) — what the
/// agent sees as "the workspace", with vendoring/build dirs pruned.
pub fn list(root: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for entry in walk(root) {
        let Ok(entry) = entry else { continue };
        if entry.file_type().is_some_and(|t| t.is_file()) {
            let path = entry.path();
            let rel = path.strip_prefix(root).unwrap_or(path);
            out.push(rel.display().to_string());
        }
    }
    out.sort();
    Ok(out)
}

/// Resolve a model-supplied relative path strictly within `base`, defeating both
/// path traversal and symlink escape. File IO runs in the trusted host process,
/// so without this a `../../.env` or a planted symlink would read/write host
/// files. `base` must already be canonical.
///
/// Rules: the path must be relative with only normal components (no `..`, no
/// root/prefix), AND the closest existing ancestor of the resolved path must
/// canonicalize to within `base` (this catches a symlink pointing outside).
pub fn safe_join(base: &Path, rel: &str) -> Result<PathBuf> {
    let p = Path::new(rel);
    if p.is_absolute() {
        bail!("path must be relative, not '{rel}'");
    }
    for comp in p.components() {
        if !matches!(comp, Component::Normal(_) | Component::CurDir) {
            bail!("path may not contain '..' or absolute segments: '{rel}'");
        }
    }
    let joined = base.join(p);
    let resolved = closest_existing(&joined)
        .canonicalize()
        .with_context(|| format!("resolving path '{rel}'"))?;
    if !resolved.starts_with(base) {
        bail!("path '{rel}' escapes the workspace");
    }
    Ok(joined)
}

/// The deepest ancestor of `p` (or `p` itself) that exists — so a not-yet-created
/// file is validated against its real parent directory.
fn closest_existing(p: &Path) -> PathBuf {
    let mut cur = p;
    loop {
        if cur.exists() {
            return cur.to_path_buf();
        }
        match cur.parent() {
            Some(parent) => cur = parent,
            None => return cur.to_path_buf(),
        }
    }
}

/// Read a `u64` from the environment, falling back to `default`.
pub fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Run a command with a wall-clock deadline, capturing output. On timeout the
/// child is killed and the result carries exit code -1 plus a timeout note — so
/// a hanging command can't wedge a run forever. stdout/stderr are drained on
/// dedicated threads so the child can't deadlock on a full pipe buffer.
pub fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Result<ExecResult> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().context("spawning command")?;

    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_handle = std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(p) = out_pipe.as_mut() {
            let _ = p.read_to_string(&mut buf);
        }
        buf
    });
    let err_handle = std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(p) = err_pipe.as_mut() {
            let _ = p.read_to_string(&mut buf);
        }
        buf
    });

    let status = child.wait_timeout(timeout).context("waiting for command")?;
    let timed_out = status.is_none();
    if timed_out {
        let _ = child.kill();
        let _ = child.wait();
    }
    let stdout = out_handle.join().unwrap_or_default();
    let mut stderr = err_handle.join().unwrap_or_default();
    let exit_code = status.and_then(|s| s.code()).unwrap_or(-1);
    if timed_out {
        stderr.push_str(&format!(
            "\n[glave] command timed out after {}s",
            timeout.as_secs()
        ));
    }
    Ok(ExecResult {
        exit_code,
        stdout,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unique, canonical temp dir to stand in for a workspace root.
    fn base(tag: &str) -> Result<PathBuf> {
        let dir = std::env::temp_dir().join(format!("glave_hostfs_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        dir.canonicalize().map_err(Into::into)
    }

    #[test]
    fn allows_normal_relative_paths() -> Result<()> {
        let b = base("ok")?;
        let p = safe_join(&b, "src/buf/buf_impl.rs")?;
        assert!(p.starts_with(&b));
        assert!(safe_join(&b, "Cargo.toml").is_ok());
        let _ = std::fs::remove_dir_all(&b);
        Ok(())
    }

    #[test]
    fn rejects_parent_traversal() -> Result<()> {
        let b = base("dotdot")?;
        assert!(safe_join(&b, "../secret").is_err());
        assert!(safe_join(&b, "../../.env").is_err());
        assert!(safe_join(&b, "a/../../b").is_err());
        let _ = std::fs::remove_dir_all(&b);
        Ok(())
    }

    #[test]
    fn rejects_absolute_paths() -> Result<()> {
        let b = base("abs")?;
        assert!(safe_join(&b, "/etc/passwd").is_err());
        let _ = std::fs::remove_dir_all(&b);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() -> Result<()> {
        let b = base("symlink")?;
        // A symlink planted inside the workspace pointing outside it.
        std::os::unix::fs::symlink("/etc", b.join("escape"))?;
        assert!(safe_join(&b, "escape/passwd").is_err());
        let _ = std::fs::remove_dir_all(&b);
        Ok(())
    }

    #[test]
    fn run_with_timeout_captures_output_and_exit_code() -> Result<()> {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo out; echo err 1>&2; exit 3"]);
        let r = run_with_timeout(cmd, Duration::from_secs(10))?;
        assert_eq!(r.exit_code, 3);
        assert!(r.stdout.contains("out"), "stdout: {}", r.stdout);
        assert!(r.stderr.contains("err"), "stderr: {}", r.stderr);
        Ok(())
    }

    #[test]
    fn run_with_timeout_kills_on_deadline() -> Result<()> {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 30"]);
        let start = std::time::Instant::now();
        let r = run_with_timeout(cmd, Duration::from_millis(300))?;
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "should have been killed promptly, took {:?}",
            start.elapsed()
        );
        assert_eq!(r.exit_code, -1);
        assert!(r.stderr.contains("timed out"), "stderr: {}", r.stderr);
        Ok(())
    }

    #[test]
    fn search_is_regex_glob_and_gitignore_aware() -> Result<()> {
        let b = base("search")?;
        std::fs::create_dir_all(b.join("src"))?;
        std::fs::write(b.join("src/lib.rs"), "fn alpha() {}\nfn beta() {}\n")?;
        std::fs::write(b.join("src/util.rs"), "fn alpha_two() {}\n")?;
        std::fs::create_dir_all(b.join("target"))?;
        std::fs::write(b.join("target/junk.rs"), "fn alpha() {}\n")?; // vendoring -> skipped
        std::fs::write(b.join(".gitignore"), "secret.rs\n")?;
        std::fs::write(b.join("secret.rs"), "fn alpha() {}\n")?; // gitignored -> skipped

        // Word boundary: matches `fn alpha()` but not `fn alpha_two()`.
        let hits = grep(&b, r"fn alpha\b", None)?;
        assert!(
            hits.iter().any(|h| h.starts_with("src/lib.rs:")),
            "hits: {hits:?}"
        );
        assert!(
            !hits.iter().any(|h| h.contains("src/util.rs")),
            "word boundary: {hits:?}"
        );
        assert!(
            !hits.iter().any(|h| h.contains("target/")),
            "vendoring dir skipped"
        );
        assert!(
            !hits.iter().any(|h| h.contains("secret.rs")),
            "gitignore respected"
        );

        let lib_only = grep(&b, "alpha", Some("**/lib.rs"))?;
        assert!(
            lib_only.iter().all(|h| h.contains("lib.rs")),
            "glob filter: {lib_only:?}"
        );

        let found = find(&b, "**/*.rs")?;
        assert!(found.iter().any(|f| f == "src/lib.rs"));
        assert!(!found
            .iter()
            .any(|f| f.contains("target/") || f.contains("secret.rs")));

        let listed = list(&b)?;
        assert!(listed.iter().any(|f| f == "src/lib.rs"));
        assert!(!listed
            .iter()
            .any(|f| f.contains("target/") || f == "secret.rs"));
        let _ = std::fs::remove_dir_all(&b);
        Ok(())
    }
}
