//! Worktrees: each branch gets its own checkout outside the repository, threads on different
//! branches never see each other's files, the same branch reopens the same worktree, and a
//! folder outside any repository is refused.

use super::helpers::{failed, files, run};
use super::{Verdict, ensure};
use crate::computer::{Command, ComputerError, ComputerId, Computers, Sources, WorkspaceInfo};

/// Runs after `moves_through_git`, which leaves a repository with one commit at `seed` in the
/// folder at `root`, as the computer names it.
pub(super) async fn worktrees<C: Computers>(
    computers: &C,
    computer: ComputerId,
    root: &str,
) -> Verdict {
    let seed = format!("{root}/seed");
    let open = |branch: &'static str| {
        let sources = Sources::Worktree {
            path: seed.clone(),
            branch: branch.into(),
        };
        async move { computers.open(computer, &sources).await }
    };
    let first = open("glave/conformance-a")
        .await
        .map_err(failed("open a worktree"))?;
    let second = open("glave/conformance-b")
        .await
        .map_err(failed("open a second worktree"))?;
    ensure(
        !first.root.starts_with(&format!("{root}/")) && first.root != second.root,
        "each worktree is its own folder, outside the repository",
    )?;
    on_branch(computers, &first, "glave/conformance-a").await?;
    computers
        .write(first.id, &files(&[("mine.txt", b"a")]))
        .await
        .map_err(failed("write in a worktree"))?;
    ensure(
        computers.read(second.id, "mine.txt").await.is_err(),
        "a worktree never sees another's files",
    )?;
    computers
        .close(first.id)
        .await
        .map_err(failed("close a worktree"))?;
    let again = open("glave/conformance-a")
        .await
        .map_err(failed("reopen a worktree"))?;
    let kept = computers.read(again.id, "mine.txt").await;
    ensure(
        again.root == first.root && kept.is_ok_and(|bytes| bytes.as_slice() == b"a"),
        "reopening a branch reopens its worktree, with its files kept",
    )?;
    let plain = Sources::Worktree {
        path: format!("{root}/plain"),
        branch: "glave/conformance-c".into(),
    };
    ensure(
        matches!(
            computers.open(computer, &plain).await,
            Err(ComputerError::Refused(_))
        ),
        "a folder outside a repository has no worktree",
    )?;
    for workspace in [again.id, second.id] {
        computers
            .close(workspace)
            .await
            .map_err(failed("close a worktree"))?;
    }
    Ok(())
}

/// Checks the worktree's checkout is on `branch`.
async fn on_branch<C: Computers>(
    computers: &C,
    workspace: &WorkspaceInfo,
    branch: &str,
) -> Verdict {
    let (_, on) = run(
        computers,
        workspace.id,
        &Command::new("git symbolic-ref --short HEAD"),
    )
    .await?;
    ensure(
        on.trim() == branch,
        "a worktree is checked out on its branch",
    )
}
