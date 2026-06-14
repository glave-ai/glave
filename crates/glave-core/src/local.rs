//! `LocalRuntime` — the on-host executor for the local client. Runs the agent
//! directly against a real folder on the user's machine: edits land **in place**
//! and git is the undo, no container. The untrusted-code sandbox (Docker /
//! microVM) is the *cloud* runtime's job; this one is for the *trusted local
//! repo*, where editing your real files is exactly what a coding tool should do.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::hostfs;
use crate::ports::{ExecResult, Workspace};

pub struct LocalRuntime {
    /// Canonical workspace root — `safe_join` requires a canonical base so
    /// model-supplied paths can't escape it via `..` or a symlink.
    root: std::path::PathBuf,
    exec_timeout: Duration,
}

impl LocalRuntime {
    /// Open `dir` as the workspace, canonicalizing it so the same path-safety
    /// guarantees as the sandbox apply.
    pub fn open(dir: &Path) -> Result<Self> {
        let root = std::fs::canonicalize(dir)
            .with_context(|| format!("opening workspace {}", dir.display()))?;
        Ok(Self {
            root,
            exec_timeout: Duration::from_secs(hostfs::env_u64("GLAVE_EXEC_TIMEOUT_SECS", 300)),
        })
    }
}

impl Workspace for LocalRuntime {
    fn read_file(&self, rel: &str) -> Result<String> {
        let p = hostfs::safe_join(&self.root, rel)?;
        std::fs::read_to_string(&p).with_context(|| format!("reading {rel}"))
    }

    fn write_file(&mut self, rel: &str, contents: &str) -> Result<()> {
        let p = hostfs::safe_join(&self.root, rel)?;
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&p, contents).with_context(|| format!("writing {rel}"))
    }

    fn list_files(&self) -> Result<Vec<String>> {
        hostfs::list(&self.root)
    }

    fn run(&self, cmd: &str) -> Result<ExecResult> {
        let mut command = Command::new("sh");
        command.current_dir(&self.root).args(["-c", cmd]);
        hostfs::run_with_timeout(command, self.exec_timeout).context("local command")
    }

    fn grep(&self, pattern: &str, file_glob: Option<&str>) -> Result<Vec<String>> {
        hostfs::grep(&self.root, pattern, file_glob)
    }

    fn find(&self, glob: &str) -> Result<Vec<String>> {
        hostfs::find(&self.root, glob)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("glave-local-{}-{name}", std::process::id()))
    }

    #[test]
    fn in_place_read_write_list_find_grep_run() -> Result<()> {
        let base = tmp("rw");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("src"))?;
        std::fs::write(base.join("src/lib.rs"), "fn alpha() {}\n")?;
        let mut rt = LocalRuntime::open(&base)?;

        // write lands in place on the real folder; read round-trips.
        rt.write_file("src/new.rs", "fn beta() {}\n")?;
        assert_eq!(
            std::fs::read_to_string(base.join("src/new.rs"))?,
            "fn beta() {}\n"
        );
        assert_eq!(rt.read_file("src/lib.rs")?, "fn alpha() {}\n");

        // list / find / grep are native + git-aware.
        assert!(rt.list_files()?.iter().any(|f| f == "src/lib.rs"));
        assert!(rt.find("**/*.rs")?.iter().any(|f| f == "src/new.rs"));
        assert!(rt
            .grep(r"fn alpha\b", None)?
            .iter()
            .any(|h| h.starts_with("src/lib.rs:1:")));

        // run executes in the workspace dir.
        let out = rt.run("ls src")?;
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("lib.rs"));

        let _ = std::fs::remove_dir_all(&base);
        Ok(())
    }

    #[test]
    fn rejects_path_escape() -> Result<()> {
        let base = tmp("escape");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base)?;
        let rt = LocalRuntime::open(&base)?;
        assert!(rt.read_file("../../../etc/passwd").is_err());
        assert!(rt.read_file("/etc/passwd").is_err());
        let _ = std::fs::remove_dir_all(&base);
        Ok(())
    }
}
