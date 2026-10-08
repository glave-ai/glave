//! The folder the tools act on: every path the model gives is confined to it, commands run
//! there without glave's secrets, and it remembers what the model has read so writes and
//! edits never act on a stale view.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use glave_event::Sha256;
use glave_ports::digest;

use crate::sandbox::{Prepared, Sandbox};
use crate::secrets::without_secrets;

/// The project folder, the environment and sandbox its commands run with, and what the model
/// has seen.
#[derive(Debug)]
pub struct Workspace {
    root: PathBuf,
    environment: Vec<(OsString, OsString)>,
    sandbox: Option<Prepared>,
    reads: Mutex<HashMap<PathBuf, Sha256>>,
}

impl Workspace {
    /// Opens a workspace at `root`, which must be an existing directory. Commands run with
    /// `environment` (usually glave's own) minus anything named like a secret, such as the
    /// model provider's API key.
    pub async fn open(
        root: impl AsRef<Path>,
        environment: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> std::io::Result<Self> {
        let root = tokio::fs::canonicalize(root).await?;
        if !tokio::fs::metadata(&root).await?.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                format!("{} is not a directory", root.display()),
            ));
        }
        Ok(Self {
            root,
            environment: without_secrets(environment),
            sandbox: None,
            reads: Mutex::new(HashMap::new()),
        })
    }

    /// Runs every command in `sandbox`. Fails if this computer cannot enforce it, so a
    /// requested sandbox is never quietly skipped.
    pub async fn with_sandbox(mut self, sandbox: &Sandbox) -> std::io::Result<Self> {
        self.sandbox = Some(sandbox.prepare(&self.root).await?);
        Ok(self)
    }

    /// The sandbox commands run in, if one was asked for.
    pub(crate) fn sandbox(&self) -> Option<&Prepared> {
        self.sandbox.as_ref()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The environment commands run with: never any secrets.
    pub(crate) fn environment(&self) -> &[(OsString, OsString)] {
        &self.environment
    }

    /// Resolves a path the model gave to one inside the workspace. Symlinks and `..` are
    /// followed before the check, so neither can escape it.
    pub(crate) async fn resolve(&self, path: &str) -> Result<PathBuf, String> {
        if path.trim().is_empty() {
            return Err("The path is empty.".into());
        }
        let candidate = self.root.join(path);
        let mut existing = candidate.as_path();
        let mut tail = Vec::new();
        while !tokio::fs::try_exists(existing).await.unwrap_or(false) {
            let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) else {
                return Err(format!("{path} cannot be resolved."));
            };
            tail.push(name.to_owned());
            existing = parent;
        }
        let mut resolved = tokio::fs::canonicalize(existing)
            .await
            .map_err(|e| format!("{path} cannot be resolved: {e}"))?;
        for name in tail.iter().rev() {
            if !matches!(
                Path::new(name).components().next(),
                Some(Component::Normal(_))
            ) {
                return Err(format!(
                    "{path} uses '..' or '.' through a folder that does not exist."
                ));
            }
            resolved.push(name);
        }
        if resolved.starts_with(&self.root) {
            Ok(resolved)
        } else {
            Err(format!(
                "{path} is outside the workspace ({}).",
                self.root.display()
            ))
        }
    }

    /// The path as the model should see it: relative to the workspace.
    pub(crate) fn show(&self, path: &Path) -> String {
        let relative = path.strip_prefix(&self.root).unwrap_or(path);
        let shown = relative.display().to_string();
        if shown.is_empty() { ".".into() } else { shown }
    }

    /// Records that the model has seen `content` as the current state of `path`.
    pub(crate) fn remember(&self, path: &Path, content: &[u8]) {
        if let Ok(mut reads) = self.reads.lock() {
            reads.insert(path.to_owned(), digest(content));
        }
    }

    /// Refuses to change a file the model has not read, or that changed since it did.
    pub(crate) fn check_fresh(&self, path: &Path, current: &[u8]) -> Result<(), String> {
        let shown = self.show(path);
        let reads = self
            .reads
            .lock()
            .map_err(|_| "The workspace's read record is unavailable.".to_owned())?;
        match reads.get(path) {
            None => Err(format!("Read {shown} before changing it.")),
            Some(seen) if *seen == digest(current) => Ok(()),
            Some(_) => Err(format!(
                "{shown} changed since you last read it. Read it again before changing it."
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("glave-ws-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
        dir
    }

    #[test]
    fn paths_stay_inside_the_workspace() {
        let dir = scratch("confine");
        block_on(async {
            let ws = Workspace::open(&dir, std::env::vars_os()).await.unwrap();
            let root = ws.root().to_owned();
            assert_eq!(
                ws.resolve("src/main.rs").await.unwrap(),
                root.join("src/main.rs")
            );
            assert_eq!(
                ws.resolve("new/dir/file.txt").await.unwrap(),
                root.join("new/dir/file.txt")
            );
            assert_eq!(
                ws.resolve(root.join("src").to_str().unwrap())
                    .await
                    .unwrap(),
                root.join("src")
            );
            assert!(ws.resolve("../outside.txt").await.is_err());
            assert!(ws.resolve("src/../../outside.txt").await.is_err());
            assert!(ws.resolve("/etc/passwd").await.is_err());
            assert!(ws.resolve("missing/../../escape").await.is_err());
            assert!(ws.resolve("  ").await.is_err());
        });
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_cannot_escape() {
        let dir = scratch("symlink");
        std::os::unix::fs::symlink(std::env::temp_dir(), dir.join("out")).unwrap();
        block_on(async {
            let ws = Workspace::open(&dir, std::env::vars_os()).await.unwrap();
            assert!(ws.resolve("out/anything").await.is_err());
        });
    }

    #[test]
    fn changes_need_a_fresh_read() {
        let dir = scratch("fresh");
        block_on(async {
            let ws = Workspace::open(&dir, std::env::vars_os()).await.unwrap();
            let file = ws.resolve("src/main.rs").await.unwrap();
            assert!(ws.check_fresh(&file, b"fn main() {}\n").is_err());
            ws.remember(&file, b"fn main() {}\n");
            assert!(ws.check_fresh(&file, b"fn main() {}\n").is_ok());
            assert!(ws.check_fresh(&file, b"changed").is_err());
        });
    }
}
