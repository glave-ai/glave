//! New projects' folders: each one made empty where the computer keeps projects, never one
//! that was there, and kept when its workspace closes; a name that is not one folder's plain
//! name is refused.

use super::helpers::{failed, files};
use super::{Verdict, ensure};
use crate::computer::{ComputerError, ComputerId, Computers, Listing, Sources};

pub(super) async fn projects<C: Computers>(computers: &C, computer: ComputerId) -> Verdict {
    let project = |name: &str| Sources::Project { name: name.into() };
    let first = computers
        .open(computer, &project("conformance-app"))
        .await
        .map_err(failed("make a project's folder"))?;
    let second = computers
        .open(computer, &project("conformance-app"))
        .await
        .map_err(failed("make a second project's folder"))?;
    ensure(
        first.root != second.root,
        "a new project's folder is never one that was there",
    )?;
    let listed = computers
        .list(second.id, &Listing::of("."))
        .await
        .map_err(failed("list a new project's folder"))?;
    ensure(listed.entries.is_empty(), "a new project's folder is empty")?;
    computers
        .write(first.id, &files(&[("plan.md", b"ship it")]))
        .await
        .map_err(failed("write in a project's folder"))?;
    for workspace in [first.id, second.id] {
        computers
            .close(workspace)
            .await
            .map_err(failed("close a project's folder"))?;
    }
    let again = computers
        .open(
            computer,
            &Sources::Folder {
                path: first.root.clone(),
            },
        )
        .await
        .map_err(failed("open a project's folder again"))?;
    let kept = computers.read(again.id, "plan.md").await;
    ensure(
        kept.is_ok_and(|bytes| bytes.as_slice() == b"ship it"),
        "closing a project's folder keeps it and its files",
    )?;
    computers
        .close(again.id)
        .await
        .map_err(failed("close a project's folder"))?;
    for name in ["../escape", ".hidden", "a/b", ""] {
        ensure(
            matches!(
                computers.open(computer, &project(name)).await,
                Err(ComputerError::Refused(_))
            ),
            "a project's folder takes only a plain name",
        )?;
    }
    Ok(())
}
