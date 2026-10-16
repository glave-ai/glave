//! What a thread opens with when it works in a folder of its own (`specs/policy.md`, "Project
//! trust"): your own AGENTS.md, then the folder's, then what its project brings that stays off.
//! The project glave was started in brings its trusted servers, hooks and settings, as it
//! always has; any other project's stay off, since this glave never started them, and the model
//! is told which and why, as it is told of untrusted ones with no one to ask.

use std::path::{Path, PathBuf};

use glave_config::policy::{Constraints, Locations};
use glave_config::trust::{self, Project};
use glave_harness::{AGENTS_MD, FolderInstructions, agents_md};
use glave_ports::BoxFuture;

use crate::setup::off_note;

/// Reads each thread's opening instructions for the folder it works in.
pub struct Opening {
    /// glave's folder: the person's AGENTS.md and the trust record.
    home: PathBuf,
    constraints: Constraints,
    /// The project glave was started in, as trust names it (its root, links resolved), and
    /// what the model is told stays off there.
    started_in: PathBuf,
    started_note: Option<String>,
}

impl Opening {
    /// Opening instructions for a glave started in the project at `started_in`, whose own
    /// setup leaves off what `started_note` tells the model.
    pub fn new(
        home: PathBuf,
        constraints: Constraints,
        started_in: &Path,
        started_note: Option<String>,
    ) -> Self {
        Self {
            started_in: project_root(&home, started_in),
            home,
            constraints,
            started_note,
        }
    }

    /// What the model is told stays off of the project whose folder is `folder`.
    async fn off(&self, folder: &Path) -> Option<String> {
        let root = project_root(&self.home, folder);
        if root == self.started_in {
            return self.started_note.clone();
        }
        let read = Project::read(&root, &crate::mcp::environment()).await;
        let off = match read {
            Ok(project) => trust::off_elsewhere(&self.home, project, &self.constraints).await,
            Err(error) => Err(error),
        };
        match off {
            Ok(lines) => off_note(&lines),
            // A project file that does not parse turns nothing on; say so rather than fail.
            Err(error) => off_note(&[format!("the project's own settings are off: {error}")]),
        }
    }
}

/// The project around `folder` as trust names it: its repository's root, or the folder
/// itself, links resolved.
fn project_root(home: &Path, folder: &Path) -> PathBuf {
    let root = Locations::here(home, folder).root;
    std::fs::canonicalize(&root).unwrap_or(root)
}

impl FolderInstructions for Opening {
    fn for_folder<'a>(
        &'a self,
        root: &'a Path,
        project: Option<&'a Path>,
    ) -> BoxFuture<'a, Option<String>> {
        Box::pin(async move {
            let personal = self.home.join(AGENTS_MD);
            let agents = match agents_md(root, Some(&personal)).await {
                Ok(read) => read.map(|read| read.text),
                Err(error) => {
                    tracing::warn!(%error, "a thread's AGENTS.md could not be read");
                    None
                }
            };
            let off = match project {
                Some(folder) => self.off(folder).await,
                None => None,
            };
            [agents, off]
                .into_iter()
                .flatten()
                .reduce(|all, part| format!("{all}\n\n{part}"))
        })
    }
}
