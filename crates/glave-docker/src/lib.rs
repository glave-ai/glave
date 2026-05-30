//! Docker `WorkspaceRuntime` adapter — the only crate that drives untrusted
//! execution, so it's also the security blast-radius boundary.
//!
//! Ephemeral container per task, workspace bind-mounted read-write and the oracle
//! read-only, no network, resource-capped. A future Firecracker/remote adapter
//! implements the same `WorkspaceRuntime` port without touching the core.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

use glave_core::{ExecResult, TaskSpec, WorkspaceRuntime};

pub struct DockerRuntime {
    container: String,
    host_work: PathBuf,
    workdir: String,
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

        let out = Command::new("docker")
            .args([
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
            ])
            .output()
            .context("docker run")?;

        if !out.status.success() {
            return Err(anyhow!(
                "docker run failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }

        Ok(Self {
            container,
            host_work: work_abs,
            workdir: task.image.workdir.clone(),
        })
    }

    pub fn cleanup(&mut self) {
        let _ = Command::new("docker")
            .args(["rm", "-f", &self.container])
            .output();
        if let Some(run_dir) = self.host_work.parent() {
            let _ = std::fs::remove_dir_all(run_dir);
        }
    }
}

impl WorkspaceRuntime for DockerRuntime {
    fn read_file(&self, rel: &str) -> Result<String> {
        let p = self.host_work.join(rel);
        std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))
    }

    fn write_file(&mut self, rel: &str, contents: &str) -> Result<()> {
        let p = self.host_work.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&p, contents).with_context(|| format!("writing {}", p.display()))
    }

    fn list_files(&self) -> Result<Vec<String>> {
        let mut out = Vec::new();
        walk(&self.host_work, &self.host_work, &mut out)?;
        Ok(out)
    }

    fn run(&self, cmd: &str) -> Result<ExecResult> {
        let out = Command::new("docker")
            .args(["exec", "-w", &self.workdir, &self.container, "sh", "-c", cmd])
            .output()
            .context("docker exec")?;
        Ok(ExecResult {
            exit_code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
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
