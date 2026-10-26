//! The behaviour every [`Computers`] adapter must show, on this machine or across the wire:
//! the lifecycle, files confined to their workspace, commands streamed and stoppable, commands
//! kept running in the background, and work leaving a computer only through git.

mod background;
mod confinement;
mod credentials;
mod finding;
mod helpers;
mod projects;
mod worktrees;

pub use confinement::{
    Confinement, confinement_case, confinement_escapes, confinement_fixture, confinement_untouched,
};
use helpers::{Collect, Trip, failed, files, run, run_quietly};

use super::{Verdict, ensure};
use crate::computer::{
    Command, ComputerError, ComputerId, Computers, Ending, EntryKind, Environment, Listing, Save,
    Sources, State, WorkspaceId,
};

/// Runs the suite on a computer made from `environment`: the host for this machine's
/// computers, an image for workers. `scratch` is an empty folder on the computer, as it names
/// it; the suite works only inside it, inside the workspaces it opens, and in the places the
/// computer keeps worktrees and new projects' folders.
pub async fn computers<C: Computers>(
    computers: &C,
    environment: &Environment,
    scratch: &str,
) -> Verdict {
    let made = computers
        .create(environment)
        .await
        .map_err(failed("create a host computer"))?;
    ensure(
        made.state == State::Running,
        "a new computer must be running",
    )?;
    let status = computers.status(made.id).await.map_err(failed("status"))?;
    ensure(
        status.id == made.id && status.state == State::Running,
        "status must describe the computer asked about",
    )?;
    let here = computers
        .open(
            made.id,
            &Sources::Folder {
                path: scratch.to_owned(),
            },
        )
        .await
        .map_err(failed("open a folder"))?;
    ensure(here.computer == made.id, "a workspace names its computer")?;
    file_operations(computers, here.id).await?;
    finding::finding(computers, here.id).await?;
    commands(computers, here.id, &here.root).await?;
    background::background(computers, made.id, here.id, scratch).await?;
    credentials::credentials(computers, here.id).await?;
    finding::sandboxed(computers, here.id).await?;
    moves_through_git(computers, made.id, here.id, &here.root).await?;
    plain_folders(computers, made.id, here.id, scratch).await?;
    worktrees::worktrees(computers, made.id, &here.root).await?;
    projects::projects(computers, made.id).await?;
    confinement::confinement(computers, made.id, here.id, scratch).await?;
    computers
        .close(here.id)
        .await
        .map_err(failed("close a folder"))?;
    let reopened = computers
        .open(
            made.id,
            &Sources::Folder {
                path: scratch.to_owned(),
            },
        )
        .await
        .map_err(failed("reopen a folder"))?;
    let kept = computers.read(reopened.id, "a.txt").await;
    ensure(
        kept.is_ok_and(|bytes| bytes.as_slice() == b"hello"),
        "closing a folder worked on in place must keep its files",
    )?;
    lifecycle_ends(computers, made.id, reopened.id).await
}

async fn file_operations<C: Computers>(computers: &C, here: WorkspaceId) -> Verdict {
    computers
        .write(
            here,
            &files(&[("a.txt", b"hello"), ("nested/b.bin", &[0, 255, 1])]),
        )
        .await
        .map_err(failed("write files"))?;
    let binary = computers
        .read(here, "nested/b.bin")
        .await
        .map_err(failed("read a file"))?;
    ensure(
        binary.as_slice() == [0, 255, 1],
        "files must read back byte for byte",
    )?;
    ensure(
        matches!(
            computers.read(here, "missing.txt").await,
            Err(ComputerError::NotFound(_))
        ),
        "reading a missing file must be not found",
    )?;
    for outside in ["../escape.txt", "/etc/hostname", "nested/../../escape.txt"] {
        ensure(
            matches!(
                computers.read(here, outside).await,
                Err(ComputerError::Refused(_))
            ),
            "reading outside the workspace must be refused",
        )?;
    }
    let mixed = files(&[("first.txt", b"1"), ("../escape.txt", b"2")]);
    ensure(
        matches!(
            computers.write(here, &mixed).await,
            Err(ComputerError::Refused(_))
        ),
        "writing outside the workspace must be refused",
    )?;
    ensure(
        computers.read(here, "first.txt").await.is_err(),
        "a refused write must write nothing",
    )?;
    listings(computers, here).await
}

async fn listings<C: Computers>(computers: &C, here: WorkspaceId) -> Verdict {
    let top = computers
        .list(here, &Listing::of("."))
        .await
        .map_err(failed("list the root"))?;
    let names: Vec<(&str, EntryKind)> = top
        .entries
        .iter()
        .map(|entry| (entry.path.as_str(), entry.kind))
        .collect();
    ensure(
        names == [("a.txt", EntryKind::File), ("nested", EntryKind::Folder)],
        "a listing holds the folder's entries, sorted, with their kinds",
    )?;
    let all = computers
        .list(
            here,
            &Listing {
                recursive: true,
                ..Listing::of(".")
            },
        )
        .await
        .map_err(failed("list everything"))?;
    ensure(
        all.entries
            .iter()
            .any(|entry| entry.path == "nested/b.bin" && entry.size == 3)
            && !all.truncated,
        "a recursive listing reaches nested files, with sizes",
    )?;
    ensure(
        matches!(
            computers.list(here, &Listing::of("..")).await,
            Err(ComputerError::Refused(_))
        ),
        "listing outside the workspace must be refused",
    )
}

async fn commands<C: Computers>(computers: &C, here: WorkspaceId, root: &str) -> Verdict {
    let (finished, output) = run(
        computers,
        here,
        &Command::new("printf 'out\\n'; printf 'err\\n' >&2; exit 3"),
    )
    .await?;
    ensure(
        finished.ending == Ending::Exited { code: 3 },
        "a command's exit code must come back",
    )?;
    ensure(
        output.contains("out\n") && output.contains("err\n"),
        "a command's output and errors must both stream back",
    )?;
    let (_, at) = run(computers, here, &Command::new("pwd -P")).await?;
    ensure(at.trim_end() == root, "commands run at the workspace root")?;
    let trip = Trip::default();
    let started = Collect::tripping(&trip, "started");
    let cancelled = computers
        .run(
            here,
            &Command::new("echo started; sleep 30"),
            &started,
            &trip,
        )
        .await
        .map_err(failed("run a command to cancel"))?;
    ensure(
        cancelled.ending == Ending::Cancelled && cancelled.millis < 20_000,
        "cancelling must stop a running command promptly",
    )?;
    let mut slow = Command::new("sleep 30");
    slow.timeout_secs = Some(1);
    let (timed, _) = run(computers, here, &slow).await?;
    ensure(
        timed.ending == Ending::TimedOut { after_secs: 1 },
        "a command past its timeout must be stopped",
    )
}

/// A bare repository, `origin.git`, with one commit on `main`.
const SEED: &str = "git init -q --bare --initial-branch=main origin.git && \
    git init -q --initial-branch=main seed && cd seed && \
    git -c user.name=seed -c user.email=seed@example.com commit -q --allow-empty -m seed && \
    git push -q ../origin.git main";

/// One workspace saves to a branch; a workspace opened from that branch sees the work.
async fn moves_through_git<C: Computers>(
    computers: &C,
    computer: ComputerId,
    here: WorkspaceId,
    root: &str,
) -> Verdict {
    run_quietly(computers, here, SEED).await?;
    let url = format!("{root}/origin.git");
    let first = computers
        .open(
            computer,
            &Sources::Git {
                url: url.clone(),
                at: Some("main".into()),
            },
        )
        .await
        .map_err(failed("open a repository"))?;
    ensure(first.root != root, "a repository opens as a new copy")?;
    computers
        .write(first.id, &files(&[("work.txt", b"moved")]))
        .await
        .map_err(failed("write in a copy"))?;
    let saved = computers
        .save(
            first.id,
            &Save {
                branch: "glave/conformance".into(),
                message: "work".into(),
            },
        )
        .await
        .map_err(failed("save a copy"))?;
    ensure(
        saved.pushed && saved.commit.len() == 40,
        "saving a copy pushes a commit to where it came from",
    )?;
    let bad = Save {
        branch: "bad..name".into(),
        message: "x".into(),
    };
    ensure(
        matches!(
            computers.save(first.id, &bad).await,
            Err(ComputerError::Refused(_))
        ),
        "an invalid branch name must be refused",
    )?;
    let second = computers
        .open(
            computer,
            &Sources::Git {
                url,
                at: Some("glave/conformance".into()),
            },
        )
        .await
        .map_err(failed("open the saved branch"))?;
    let moved = computers
        .read(second.id, "work.txt")
        .await
        .map_err(failed("read the moved work"))?;
    ensure(
        moved.as_slice() == b"moved",
        "saved work must reach another copy",
    )?;
    for copy in [first.id, second.id] {
        computers
            .close(copy)
            .await
            .map_err(failed("close a copy"))?;
    }
    ensure(
        matches!(
            computers.read(first.id, "work.txt").await,
            Err(ComputerError::NotFound(_))
        ),
        "a closed workspace is not found",
    )
}

/// A folder that is not a repository is worked in place: saving it is refused, and glave
/// writes no history into it, neither a `.git` nor a hidden one.
async fn plain_folders<C: Computers>(
    computers: &C,
    computer: ComputerId,
    here: WorkspaceId,
    scratch: &str,
) -> Verdict {
    computers
        .write(here, &files(&[("plain/notes.txt", b"one")]))
        .await
        .map_err(failed("write a plain folder"))?;
    let plain = computers
        .open(
            computer,
            &Sources::Folder {
                path: format!("{scratch}/plain"),
            },
        )
        .await
        .map_err(failed("open a plain folder"))?;
    let save = Save {
        branch: "glave/notes".into(),
        message: "notes".into(),
    };
    ensure(
        matches!(
            computers.save(plain.id, &save).await,
            Err(ComputerError::Refused(_))
        ),
        "saving a folder that is not a repository is refused",
    )?;
    let listed = computers
        .list(
            plain.id,
            &Listing {
                recursive: true,
                ..Listing::of(".")
            },
        )
        .await
        .map_err(failed("list a plain folder"))?;
    ensure(
        !listed.entries.iter().any(|entry| {
            matches!(
                entry.path.as_str(),
                ".git" | ".glave" | ".glave/history.git"
            )
        }),
        "glave must write no history into a plain folder",
    )?;
    computers
        .close(plain.id)
        .await
        .map_err(failed("close a plain folder"))
}

async fn lifecycle_ends<C: Computers>(
    computers: &C,
    computer: ComputerId,
    workspace: WorkspaceId,
) -> Verdict {
    computers
        .destroy(computer)
        .await
        .map_err(failed("destroy"))?;
    let gone = computers
        .status(computer)
        .await
        .map_err(failed("status after destroy"))?;
    ensure(gone.state == State::Gone, "a destroyed computer is gone")?;
    ensure(
        matches!(
            computers.read(workspace, "a.txt").await,
            Err(ComputerError::NotFound(_))
        ),
        "destroying a computer closes its workspaces",
    )?;
    ensure(
        matches!(
            computers
                .open(computer, &Sources::Folder { path: ".".into() })
                .await,
            Err(ComputerError::NotFound(_))
        ),
        "a destroyed computer opens nothing",
    )
}
