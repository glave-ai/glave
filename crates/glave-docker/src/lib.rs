//! Docker `WorkspaceRuntime` adapter — the only crate that drives untrusted
//! execution, so it's also the security blast-radius boundary.
//!
//! Ephemeral container per task, workspace bind-mounted read-write and the oracle
//! read-only, no network, resource-capped. A future Firecracker/remote adapter
//! implements the same `WorkspaceRuntime` port without touching the core.

use anyhow::{anyhow, bail, Context, Result};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use wait_timeout::ChildExt;

use glave_core::{ExecResult, RuntimeFactory, TaskSpec, WorkspaceRuntime};

pub struct DockerRuntime {
    container: String,
    host_work: PathBuf,
    workdir: String,
    exec_timeout: Duration,
}

impl DockerRuntime {
    pub fn prepare(task: &TaskSpec, run_id: &str) -> Result<Self> {
        let cwd = std::env::current_dir()?;
        let host_work = cwd.join(".runs").join(run_id).join("work");
        std::fs::create_dir_all(&host_work)?;
        copy_dir(&task.workspace_dir(), &host_work).context("copying workspace into run dir")?;

        let work_abs = std::fs::canonicalize(&host_work)?;
        let oracle_abs = std::fs::canonicalize(task.oracle_dir())
            .with_context(|| format!("canonicalizing {}", task.oracle_dir().display()))?;

        let container = format!("glave_{}", run_id.replace('-', ""));
        let mem = format!("{}m", task.resource.mem_mb);
        let cpus = task.resource.cpus.to_string();
        let pids = task.resource.pids.to_string();
        let network = secure_network(&task.resource.network);
        let work_mount = format!("{}:{}", work_abs.display(), task.image.workdir);
        let oracle_mount = format!("{}:/oracle:ro", oracle_abs.display());

        let mut run_cmd = Command::new("docker");
        run_cmd.args([
            "run",
            "-d",
            "--name",
            &container,
            "--network",
            &network,
            "--memory",
            &mem,
            "--memory-swap",
            &mem, // no swap: OOM-kill rather than eat the VM
            "--cpus",
            &cpus,
            "--pids-limit",
            &pids,
            "--cap-drop",
            "ALL",
            "--security-opt",
            "no-new-privileges",
            "-v",
            &work_mount,
            "-v",
            &oracle_mount,
            "-w",
            &task.image.workdir,
            &task.image.base,
            "tail",
            "-f",
            "/dev/null",
        ]);
        // Bounded so a stuck image pull / daemon hang can't wedge a run forever.
        let run_timeout = Duration::from_secs(env_u64("GLAVE_RUN_TIMEOUT_SECS", 120));
        let out = run_with_timeout(run_cmd, run_timeout).context("docker run")?;
        if out.exit_code != 0 {
            return Err(anyhow!("docker run failed: {}", out.stderr.trim()));
        }

        Ok(Self {
            container,
            host_work: work_abs,
            workdir: task.image.workdir.clone(),
            exec_timeout: Duration::from_secs(env_u64("GLAVE_EXEC_TIMEOUT_SECS", 300)),
        })
    }

}

impl Drop for DockerRuntime {
    /// Best-effort RAII cleanup: kill the container and remove the run dir. Runs
    /// at the end of every run — including the error path — so a failed run can
    /// never leak a container or a workspace copy.
    fn drop(&mut self) {
        // -v also reaps any anonymous volume, so a run leaves nothing behind
        // (we bind-mount rather than use volumes, but this guarantees it).
        let _ = Command::new("docker")
            .args(["rm", "-f", "-v", &self.container])
            .output();
        if let Some(run_dir) = self.host_work.parent() {
            let _ = std::fs::remove_dir_all(run_dir);
        }
    }
}

/// Mints a fresh ephemeral Docker sandbox per run.
pub struct DockerRuntimeFactory;

impl RuntimeFactory for DockerRuntimeFactory {
    fn prepare(&self, task: &TaskSpec, run_id: &str) -> Result<Box<dyn WorkspaceRuntime>> {
        Ok(Box::new(DockerRuntime::prepare(task, run_id)?))
    }
}

impl WorkspaceRuntime for DockerRuntime {
    fn read_file(&self, rel: &str) -> Result<String> {
        let p = safe_join(&self.host_work, rel)?;
        std::fs::read_to_string(&p).with_context(|| format!("reading {rel}"))
    }

    fn write_file(&mut self, rel: &str, contents: &str) -> Result<()> {
        let p = safe_join(&self.host_work, rel)?;
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&p, contents).with_context(|| format!("writing {rel}"))
    }

    fn list_files(&self) -> Result<Vec<String>> {
        let mut out = Vec::new();
        walk(&self.host_work, &self.host_work, &mut out)?;
        Ok(out)
    }

    fn run(&self, cmd: &str) -> Result<ExecResult> {
        let mut command = Command::new("docker");
        command.args(["exec", "-w", &self.workdir, &self.container, "sh", "-c", cmd]);
        run_with_timeout(command, self.exec_timeout).context("docker exec")
    }

    // Search runs natively over the host bind-mount (no `rg` in the sandbox), so
    // it's self-sufficient and identical to what the local client will run.
    fn grep(&self, pattern: &str, file_glob: Option<&str>) -> Result<Vec<String>> {
        search::grep(&self.host_work, pattern, file_glob)
    }

    fn find(&self, glob: &str) -> Result<Vec<String>> {
        search::find(&self.host_work, glob)
    }
}

/// Native, git-aware code search via ripgrep's own crates — no `rg` binary.
/// Respects `.gitignore` and skips vendoring/build dirs, so the agent searches
/// real source like a developer would. The same logic will run client-side.
mod search {
    use anyhow::{anyhow, Result};
    use globset::Glob;
    use grep::regex::RegexMatcher;
    use grep::searcher::sinks::UTF8;
    use grep::searcher::SearcherBuilder;
    use ignore::WalkBuilder;
    use std::path::Path;

    const SKIP: [&str; 6] = ["vendor", "target", "node_modules", ".git", ".cargo", ".venv"];

    fn walk(root: &Path) -> ignore::Walk {
        WalkBuilder::new(root)
            .hidden(false) // show dotfiles; .gitignore + SKIP still prune
            .require_git(false) // honor .gitignore even before the repo is a git checkout
            .filter_entry(|e| e.file_name().to_str().is_none_or(|n| !SKIP.contains(&n)))
            .build()
    }

    pub fn grep(root: &Path, pattern: &str, file_glob: Option<&str>) -> Result<Vec<String>> {
        let matcher =
            RegexMatcher::new(pattern).map_err(|e| anyhow!("invalid search pattern: {e}"))?;
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

    pub fn find(root: &Path, glob: &str) -> Result<Vec<String>> {
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
}

/// Resolve a model-supplied relative path strictly within `base`, defeating both
/// path traversal and symlink escape. The model controls these paths, and the
/// file IO runs in the trusted host process (not the container) — so without
/// this, `../../.env` or a symlink planted via `<bash>` would read/write host
/// files. `base` must already be canonical (it is: `prepare` canonicalizes it).
///
/// Rules: the path must be relative with only normal components (no `..`, no
/// root/prefix), AND the closest existing ancestor of the resolved path must
/// canonicalize to within `base` (this catches a symlink that points outside).
fn safe_join(base: &Path, rel: &str) -> Result<PathBuf> {
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

/// The deepest ancestor of `p` (or `p` itself) that exists on disk — so a
/// not-yet-created file is validated against its real parent directory.
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

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Run a command with a wall-clock deadline, capturing output. On timeout the
/// child is killed and the result carries exit code -1 plus a timeout note — so
/// a model command that hangs (`sleep infinity`, an infinite-loop test) can't
/// wedge a run forever; it surfaces as a failed command the agent/oracle handles.
///
/// stdout/stderr are drained on dedicated threads so the child can't deadlock on
/// a full pipe buffer while we wait.
fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Result<ExecResult> {
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
        stderr.push_str(&format!("\n[glave] command timed out after {}s", timeout.as_secs()));
    }
    Ok(ExecResult { exit_code, stdout, stderr })
}

/// Fail-closed network policy. The no-exfil guarantee must not depend on the
/// task file being honest, so a non-"none" request is honored only with an
/// explicit operator opt-in (`GLAVE_ALLOW_NETWORK`); otherwise it's forced off.
fn secure_network(requested: &str) -> String {
    if requested == "none" {
        return "none".to_string();
    }
    if std::env::var("GLAVE_ALLOW_NETWORK").is_ok() {
        return requested.to_string();
    }
    eprintln!(
        "WARN: task requested network={requested:?}; forcing 'none' (set GLAVE_ALLOW_NETWORK to override)"
    );
    "none".to_string()
}

fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src).with_context(|| format!("reading {}", src.display()))? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
    const SKIP: [&str; 5] = ["vendor", "target", "node_modules", ".git", ".cargo"];
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_dir() {
            let name = p
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            if SKIP.contains(&name.as_str()) {
                continue;
            }
            walk(root, &p, out)?;
        } else if let Ok(rel) = p.strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unique, canonical temp dir to stand in for a run's `host_work`.
    fn base(tag: &str) -> Result<PathBuf> {
        let dir = std::env::temp_dir().join(format!("glave_safejoin_{}_{tag}", std::process::id()));
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
        // Plant a symlink inside the workspace pointing outside it (the kind a
        // model could create via `<bash> ln -s` through the bind mount).
        std::os::unix::fs::symlink("/etc", b.join("escape"))?;
        // Reading through it must be rejected — it canonicalizes outside `base`.
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
        std::fs::write(b.join("target/junk.rs"), "fn alpha() {}\n")?; // vendoring dir -> skipped
        std::fs::write(b.join(".gitignore"), "secret.rs\n")?;
        std::fs::write(b.join("secret.rs"), "fn alpha() {}\n")?; // gitignored -> skipped

        // Regex with a word boundary: matches `fn alpha()` but not `fn alpha_two()`.
        let hits = search::grep(&b, r"fn alpha\b", None)?;
        assert!(hits.iter().any(|h| h.starts_with("src/lib.rs:")), "hits: {hits:?}");
        assert!(!hits.iter().any(|h| h.contains("src/util.rs")), "word boundary: {hits:?}");
        assert!(!hits.iter().any(|h| h.contains("target/")), "vendoring dir skipped");
        assert!(!hits.iter().any(|h| h.contains("secret.rs")), "gitignore respected");

        // Glob filter restricts which files are searched.
        let lib_only = search::grep(&b, "alpha", Some("**/lib.rs"))?;
        assert!(lib_only.iter().all(|h| h.contains("lib.rs")), "glob filter: {lib_only:?}");

        let found = search::find(&b, "**/*.rs")?;
        assert!(found.iter().any(|f| f == "src/lib.rs"));
        assert!(!found.iter().any(|f| f.contains("target/") || f.contains("secret.rs")));
        let _ = std::fs::remove_dir_all(&b);
        Ok(())
    }
}
