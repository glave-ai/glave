//! Bringing a thread's copy of its project up to the project's current commit, before a
//! schedule's run works on it, so "check the build every morning" checks today's build. Only a
//! clean copy moves, and only forward: one with changes of its own, or commits of its own where
//! the project moved on too, is left as it is, and the run is told why in its next turn's opening
//! note, which never changes the person's words or the thread's prefix.

use glave_ports::computer::{OnTimeout, Sources};
use glave_ports::{BlobStore, EventLog, ModelProvider, NeverCancelled};

use crate::approval::Approver;
use crate::harness::{Harness, HarnessError, Thread};
use crate::tools::workspace::Collected;

/// The longest bringing a copy up to date may take.
const SECONDS: u64 = 30;

/// Moves a clean worktree forward to the commit its repository's own folder is at, saying what
/// came of it: `current` when it has that commit now, `changed` when it has uncommitted changes,
/// `diverged` when it has commits of its own and the project moved on, `failed` otherwise.
const CATCH_UP: &str = r#"if [ -n "$(git status --porcelain)" ]; then echo changed; exit 0; fi
base=$(git worktree list --porcelain | awk 'NR == 2 { print $2 }')
if [ -z "$base" ]; then echo failed; exit 0; fi
if git merge-base --is-ancestor "$base" HEAD; then echo current; exit 0; fi
git merge-base --is-ancestor HEAD "$base"
case $? in 0) ;; 1) echo diverged; exit 0 ;; *) echo failed; exit 0 ;; esac
if git merge --ff-only --quiet "$base" >/dev/null 2>&1; then echo current; else echo failed; fi"#;

/// What bringing a thread's copy up to date came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaughtUp {
    /// The thread works on no copy of its project.
    NoCopy,
    /// Its copy has the project's current commit now.
    Current,
    /// Its copy has changes of its own not yet committed, so it was left as it was.
    Changed,
    /// Its copy has commits of its own and the project moved on, so it was left as it was.
    Diverged,
    /// Bringing it up to date failed, so it may be behind.
    Failed,
}

impl Thread {
    /// Tells the model `note` in the opening note of this thread's next turn, after what changed
    /// since it was last told: never in the person's words, nor in the prefix.
    pub fn remind(&mut self, note: String) {
        self.reminder = Some(note);
    }
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
        Ok(match output.text().trim_end().lines().last() {
            Some("current") => CaughtUp::Current,
            Some("changed") => CaughtUp::Changed,
            Some("diverged") => CaughtUp::Diverged,
            _ => CaughtUp::Failed,
        })
    }
}
