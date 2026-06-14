//! Docker `Workspace`/`Grader` adapter — the only crate that drives untrusted
//! execution, so it's also the security blast-radius boundary.
//!
//! Ephemeral container per task, workspace bind-mounted read-write and the oracle
//! read-only, no network, resource-capped. A future Firecracker/remote adapter
//! implements the same `Workspace`/`Grader` ports without touching the core.
//!
//! Host-side primitives — path safety, command exec, git-aware search — live in
//! `glave_core::hostfs`, shared verbatim with the local client's `LocalRuntime`
//! so the two runtimes can never drift apart on those behaviors.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use glave_core::hostfs;
use glave_core::{EvalRuntime, ExecResult, Grader, RuntimeFactory, TaskSpec, Workspace};

pub struct DockerRuntime {
    container: String,
    host_work: PathBuf,
    workdir: String,
    exec_timeout: Duration,
    /// What `grade` needs to spin its own ephemeral container: the agent's
    /// container deliberately has NO oracle mount (the held tests must be
    /// unreachable during the run), so grading runs separately.
    image: String,
    oracle_dir: PathBuf,
}

/// The security hardening every container we spawn must share — the sandbox
/// boundary. Applied to both the agent's run container and the ephemeral grade
/// container so the two can never drift apart.
fn harden(cmd: &mut Command) {
    cmd.args(["--cap-drop", "ALL", "--security-opt", "no-new-privileges"]);
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

        // NOTE: the agent's container gets NO /oracle mount — the held tests and
        // grading scripts must be unreachable during the run (a model has been
        // observed going looking for them). Grading runs in a separate ephemeral
        // container; see `grade`.
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
        ]);
        harden(&mut run_cmd);
        run_cmd.args([
            "-v",
            &work_mount,
            "-w",
            &task.image.workdir,
            &task.image.base,
            "tail",
            "-f",
            "/dev/null",
        ]);
        // Bounded so a stuck image pull / daemon hang can't wedge a run forever.
        let run_timeout = Duration::from_secs(hostfs::env_u64("GLAVE_RUN_TIMEOUT_SECS", 120));
        let out = hostfs::run_with_timeout(run_cmd, run_timeout).context("docker run")?;
        if out.exit_code != 0 {
            return Err(anyhow!("docker run failed: {}", out.stderr.trim()));
        }

        Ok(Self {
            container,
            host_work: work_abs,
            workdir: task.image.workdir.clone(),
            exec_timeout: Duration::from_secs(hostfs::env_u64("GLAVE_EXEC_TIMEOUT_SECS", 300)),
            image: task.image.base.clone(),
            oracle_dir: oracle_abs,
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
    fn prepare(&self, task: &TaskSpec, run_id: &str) -> Result<Box<dyn EvalRuntime>> {
        Ok(Box::new(DockerRuntime::prepare(task, run_id)?))
    }
}

impl Workspace for DockerRuntime {
    fn read_file(&self, rel: &str) -> Result<String> {
        let p = hostfs::safe_join(&self.host_work, rel)?;
        std::fs::read_to_string(&p).with_context(|| format!("reading {rel}"))
    }

    fn write_file(&mut self, rel: &str, contents: &str) -> Result<()> {
        let p = hostfs::safe_join(&self.host_work, rel)?;
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&p, contents).with_context(|| format!("writing {rel}"))
    }

    fn list_files(&self) -> Result<Vec<String>> {
        hostfs::list(&self.host_work)
    }

    fn run(&self, cmd: &str) -> Result<ExecResult> {
        let mut command = Command::new("docker");
        command.args([
            "exec",
            "-w",
            &self.workdir,
            &self.container,
            "sh",
            "-c",
            cmd,
        ]);
        hostfs::run_with_timeout(command, self.exec_timeout).context("docker exec")
    }

    // Search runs natively over the host bind-mount (no `rg` in the sandbox), so
    // it's self-sufficient and identical to what the local client runs.
    fn grep(&self, pattern: &str, file_glob: Option<&str>) -> Result<Vec<String>> {
        hostfs::grep(&self.host_work, pattern, file_glob)
    }

    fn find(&self, glob: &str) -> Result<Vec<String>> {
        hostfs::find(&self.host_work, glob)
    }
}

/// Grade-time oracle execution — kept off [`Workspace`] so the agent (which only
/// ever holds a `Workspace`) can never reach the held tests. With this, the
/// `DockerRuntime` is a full `EvalRuntime` (workspace + grader) for the eval path.
impl Grader for DockerRuntime {
    /// A FRESH ephemeral container (`--rm`) that mounts the agent's edited
    /// workspace plus /oracle read-only. The agent's own container never sees
    /// /oracle, so the held tests are unreachable during the run; this one exists
    /// only after the agent is done, for one command, with `--network none`
    /// (grading is deterministic and never needs egress, regardless of what the
    /// agent run was allowed). NOTE: this is a fresh container — any env the agent
    /// `export`ed mid-run is gone; the oracle commands must be self-contained
    /// (ours invoke `.venv/bin/python` directly).
    fn grade(&self, cmd: &str) -> Result<ExecResult> {
        let work_mount = format!("{}:{}", self.host_work.display(), self.workdir);
        let oracle_mount = format!("{}:/oracle:ro", self.oracle_dir.display());
        let mut command = Command::new("docker");
        command.args(["run", "--rm", "--network", "none"]);
        harden(&mut command);
        command.args([
            "-v",
            &work_mount,
            "-v",
            &oracle_mount,
            "-w",
            &self.workdir,
            &self.image,
            "sh",
            "-c",
            cmd,
        ]);
        hostfs::run_with_timeout(command, self.exec_timeout).context("docker grade run")
    }
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
        // `file_type()` here does NOT follow symlinks (it's the entry's own type).
        let ft = entry.file_type()?;
        if ft.is_symlink() {
            // Preserve symlinks verbatim. An in-container venv symlinks
            // `bin/python` to the container interpreter — a target that doesn't
            // exist on the host but is valid again once `/work` is remounted.
            // Following it (the old behavior) errored with "No such file".
            let target = std::fs::read_link(&from)?;
            let _ = std::fs::remove_file(&to);
            std::os::unix::fs::symlink(&target, &to)?;
        } else if ft.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}
