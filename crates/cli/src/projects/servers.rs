//! Each project's servers (`specs/one-server.md`, "Each project's own tools, hooks and
//! settings"): started in its folder the first time one of its turns needs them, kept for its
//! later turns while they are the same, and closed after 30 minutes in which no turn of the
//! project used them. At most 16 projects' run at once, the one used least recently closed
//! first, and a set a turn still holds closes once the turn lets go of it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use glave_config::policy::Constraints;
use glave_mcp::McpServer;
use glave_ports::ToolSource;
use glave_process::Processes;
use jiff::{SignedDuration, Timestamp};

/// The projects whose servers run at once for their threads; past this, the one used least
/// recently is closed, and started again when its next turn needs it.
const MOST_PROJECTS: usize = 16;

/// How long a project's servers stay up with no turn of the project using them, in seconds.
const IDLE_SECS: i64 = 30 * 60;

/// The servers started for one project's threads.
pub(super) struct Kept {
    pub(super) root: PathBuf,
    /// What was trusted, by name, each as JSON: started again when it changes.
    pub(super) servers: BTreeMap<String, String>,
    pub(super) started: Vec<Arc<McpServer>>,
    pub(super) sources: Vec<Arc<dyn ToolSource>>,
    /// When a turn of the project last used them.
    pub(super) used: Timestamp,
}

impl Kept {
    /// Whether a thread may still use these servers: its turn's tools hold their sources.
    fn in_use(&self) -> bool {
        self.sources
            .last()
            .is_some_and(|resources| Arc::strong_count(resources) > 1)
    }
}

/// The servers started for projects' threads.
#[derive(Default)]
pub(super) struct Running {
    /// Each project's, most recently used last.
    pub(super) kept: Vec<Kept>,
    /// Replaced or put away while a thread still held them: closed once none does.
    pub(super) retired: Vec<Kept>,
}

impl Running {
    /// Puts `kept` away, and takes out what no thread uses any more, to close.
    pub(super) fn retire(&mut self, kept: impl IntoIterator<Item = Kept>) -> Vec<Kept> {
        self.retired.extend(kept);
        let (busy, idle) = std::mem::take(&mut self.retired)
            .into_iter()
            .partition(Kept::in_use);
        self.retired = busy;
        idle
    }

    /// Takes out, to close, every project's servers no turn has used for 30 minutes by `now`,
    /// and those put away that no turn holds any more. A set a turn holds is in use now.
    pub(super) fn idle(&mut self, now: Timestamp) -> Vec<Kept> {
        for kept in &mut self.kept {
            if kept.in_use() {
                kept.used = now;
            }
        }
        let idle = SignedDuration::from_secs(IDLE_SECS);
        let (gone, kept): (Vec<Kept>, Vec<Kept>) = std::mem::take(&mut self.kept)
            .into_iter()
            .partition(|kept| now.duration_since(kept.used) >= idle);
        self.kept = kept;
        self.retire(gone)
    }

    /// The tool sources of the project at `root` for `servers` at `now`: kept from an earlier
    /// turn while they are the same, else started now under `constraints`; and what they
    /// replace, and a project's past [`MOST_PROJECTS`], to close, once no thread's turn holds
    /// it any more.
    pub(super) fn sources(
        &mut self,
        root: &Path,
        servers: BTreeMap<String, String>,
        now: Timestamp,
        constraints: &Constraints,
        processes: &Processes,
    ) -> (Vec<Arc<dyn ToolSource>>, Vec<Kept>) {
        let mut closing = Vec::new();
        if let Some(at) = self.kept.iter().position(|kept| kept.root == root) {
            let mut kept = self.kept.remove(at);
            if kept.servers == servers {
                kept.used = now;
                let sources = kept.sources.clone();
                self.kept.push(kept);
                return (sources, self.retire([]));
            }
            closing.push(kept);
        }
        let sources = if servers.is_empty() {
            Vec::new()
        } else {
            match crate::mcp::start_project(&servers, root, constraints, processes) {
                Ok(started) => {
                    let sources = crate::mcp::sources(&started);
                    self.kept.push(Kept {
                        root: root.to_owned(),
                        servers,
                        started,
                        sources: sources.clone(),
                        used: now,
                    });
                    sources
                }
                Err(error) => {
                    tracing::warn!(%error, "a project's trusted servers could not start");
                    Vec::new()
                }
            }
        };
        let over = self.kept.len().saturating_sub(MOST_PROJECTS);
        closing.extend(self.kept.drain(..over));
        (sources, self.retire(closing))
    }
}

/// Closes `kept`, each set's servers side by side.
pub(super) async fn close(kept: impl IntoIterator<Item = Kept>) {
    for kept in kept {
        crate::mcp::close(&kept.started).await;
    }
}
