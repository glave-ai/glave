//! Bringing a thread's copy of its project up to the project's current commit, before a
//! schedule's run works on it, so "check the build every morning" checks today's build. Only a
//! clean copy moves, and only forward: one with changes of its own, uncommitted or committed, is
//! left as it is, and the run is told so.

use glave_ports::computer::{OnTimeout, Sources};
use glave_ports::{BlobStore, EventLog, ModelProvider, NeverCancelled};

use crate::approval::Approver;
use crate::harness::{Harness, HarnessError, Thread};
use crate::tools::workspace::Collected;

/// The longest bringing a copy up to date may take.
const SECONDS: u64 = 30;

/// Moves a clean worktree forward to the commit its repository's own folder is at, saying which
/// came of it: `current` when it is there now, `kept` when it has changes of its own.
const CATCH_UP: &str = r#"if [ -n "$(git status --porcelain)" ]; then echo kept; exit 0; fi
base=$(git worktree list --porcelain | awk 'NR == 2 { print $2 }')
if git merge --ff-only --quiet "$base" >/dev/null 2>&1; then echo current; else echo kept; fi"#;

/// What bringing a thread's copy up to date came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaughtUp {
    /// The thread works on no copy of its project.
    NoCopy,
    /// Its copy is at the project's current commit now.
    Current,
    /// Its copy has changes of its own, so it was left as it was.
    Kept,
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Brings `thread`'s copy of its project up to the project's current commit, when it works
    /// on one and it is clean.
    pub async fn catch_up_copy(&self, thread: &mut Thread) -> Result<CaughtUp, HarnessError> {
        self.come_home(thread).await?;
        let workspace = self.workspace_for(thread);
        if !matches!(workspace.info().sources, Sources::Worktree { .. }) {
            return Ok(CaughtUp::NoCopy);
        }
        let output = Collected::default();
        let limit = (SECONDS, OnTimeout::Stop);
        workspace
            .run(CATCH_UP.to_owned(), limit, &output, &NeverCancelled)
            .await?;
        Ok(if output.text().trim_end().ends_with("current") {
            CaughtUp::Current
        } else {
            CaughtUp::Kept
        })
    }
}
