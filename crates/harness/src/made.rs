//! Files a turn made for the person (`specs/session.md`, "Files it made"). The thread's folder
//! is looked over (`files/glob`) for every document, spreadsheet, deck, PDF and e-book in it,
//! with each one's size and when it last changed, before the turn's first tool call and again
//! as the turn ends: what is new or changed between, in no hidden folder, counts as made,
//! whatever made it (a `write`, a script, a command run from another folder, a worker in the same
//! folder). Each is read through the computer port and kept for the thread, sealed, one at a
//! time, so it outlives the computer; the model never sees it.

use std::collections::BTreeMap;

use glave_event::{AssetKind, FileMade, Kept};
use glave_ports::computer::{EntryKind, Glob, Stamp};
use glave_ports::{BlobStore, EventLog, ModelProvider, Pieces, ToolCall, digest};

use crate::approval::Approver;
use crate::files::MAX_FILE;
use crate::harness::{Harness, Thread};
use crate::tools::Tool;
use crate::toolset::ToolSet;

/// The most files one turn keeps: past them, the rest stay on the computer, unnamed.
pub(crate) const MOST: usize = 8;

/// The most files of the kinds that count looked over in a folder.
const MOST_SEEN: usize = 10_000;

/// What the thread's folder held of the files that would count as made, by path: each one's
/// stamp, when the computer says it. `None` when the folder could not be looked over.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Seen(Option<BTreeMap<String, Option<Stamp>>>);

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// What `thread`'s folder holds now of the files that would count as made. A worker's
    /// files are its parent's, which looks over the same folder.
    pub(crate) async fn seen(&self, thread: &Thread) -> Seen {
        if thread.delegated() {
            return Seen(None);
        }
        let glob = Glob {
            pattern: AssetKind::made_pattern(),
            path: ".".to_owned(),
            limit: MOST_SEEN,
        };
        let Ok(found) = self.workspace_for(thread).glob(&glob).await else {
            return Seen(None);
        };
        let stamps = found
            .stamps
            .into_iter()
            .map(Some)
            .chain(std::iter::repeat(None));
        Seen(Some(found.files.into_iter().zip(stamps).collect()))
    }

    /// The files `thread`'s turn made since its folder held what `before` says, each kept for
    /// the thread or said why not, in path order: at most [`MOST`].
    pub(crate) async fn made_since(&self, thread: &Thread, before: Option<Seen>) -> Vec<FileMade> {
        let Some(Seen(Some(before))) = before else {
            return Vec::new();
        };
        let Seen(Some(now)) = self.seen(thread).await else {
            return Vec::new();
        };
        let mut made = Vec::new();
        let changed = now
            .iter()
            .filter(|(path, stamp)| before.get(*path) != Some(*stamp) && !hidden(path));
        for (path, _) in changed {
            if made.len() >= MOST {
                break;
            }
            if let Some(file) = self.keep_made_file(thread, path).await {
                made.push(file);
            }
        }
        made
    }

    /// The file at `path` in `thread`'s folder, kept for the thread when it is no larger than a
    /// thread keeps and holds none of the person's secrets. `None` when it isn't a plain file
    /// there now, or it changed while it was read: the next turn sees it again.
    async fn keep_made_file(&self, thread: &Thread, path: &str) -> Option<FileMade> {
        let workspace = self.workspace_for(thread);
        let stat = workspace.stat(path).await.ok()?;
        let media_type = AssetKind::made_as(&stat.path)?;
        if stat.kind != Some(EntryKind::File) {
            return None;
        }
        let made = |kept| FileMade {
            path: stat.path.clone(),
            bytes: stat.size,
            kept,
        };
        if stat.size > MAX_FILE {
            return Some(made(Kept::TooLarge));
        }
        let bytes = workspace.read(&stat.path).await.ok()?.into_vec();
        if Some(digest(&bytes)) != stat.digest {
            return None;
        }
        if self.workspace.hidden().held_in(&bytes) {
            return Some(made(Kept::Secret));
        }
        let pieces = Box::new(Pieces::from(bytes));
        let put = self
            .blobs
            .put(&thread.scope, Some(thread.id), media_type, MAX_FILE, pieces);
        match put.await {
            Ok(blob) => Some(made(Kept::Blob(blob))),
            Err(error) => {
                tracing::warn!(%error, "a file the turn made could not be kept");
                None
            }
        }
    }
}

/// Whether any of `calls` can change files in `thread`'s folder, so looking it over first wakes
/// nothing that the calls won't: glave's own `write`, `edit` or `bash`, or a worker on a
/// computer that never sleeps. A worker on a person's own computer, which may sleep, is left to
/// wake it with its own tools.
pub(crate) fn works_there(thread: &Thread, calls: &[ToolCall], toolset: &ToolSet) -> bool {
    calls.iter().any(|call| {
        matches!(
            Tool::named(&call.tool),
            Some(Tool::Write | Tool::Edit | Tool::Bash)
        ) || (thread.home.is_none() && toolset.is_agent(&call.tool))
    })
}

/// Whether `path`, relative to the thread's folder, is in a hidden folder or is hidden itself:
/// scratch, such as glave's own folder or a cache, never something made for the person.
fn hidden(path: &str) -> bool {
    path.split('/')
        .any(|name| name.starts_with('.') && name != ".")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_hidden_is_scratch() {
        assert!(hidden(".glave/out.pdf") && hidden("work/.cache/a.xlsx") && hidden(".a.pdf"));
        assert!(!hidden("out/report.pdf") && !hidden("report.pdf"));
    }
}
