//! The targeted escapes: links a command plants, trailing slashes, a hard link to a file
//! outside, links into protected paths; and, so the refusals cannot pass by refusing
//! everything, links that stay inside working and a replaced file keeping its permissions.

use super::super::helpers::{failed, run, run_quietly};
use super::{Confinement, LINKED, WRITTEN, outside_unchanged, plain};
use crate::computer::{
    Command, ComputerError, Computers, FileWrite, Listing, is_protected, refuse_protected,
};
use crate::conformance::{Verdict, ensure};

pub async fn confinement_escapes<C: Computers>(computers: &C, fixture: &Confinement) -> Verdict {
    let here = fixture.workspace;
    run_quietly(
        computers,
        here,
        "ln -sf ../outside/sentinel.txt planted && ln -sfn ../outside planted_dir && \
         ln -f ../outside/linked.txt hard.txt",
    )
    .await?;
    planted(computers, fixture).await?;
    hard_link(computers, fixture).await?;
    links_inside(computers, fixture).await?;
    permissions(computers, fixture).await?;
    let long = computers.write(here, &[plain(&fixture.long, b"x")]).await;
    ensure(
        !matches!(
            long,
            Err(ComputerError::Unreachable(_) | ComputerError::Unsupported(_))
        ),
        "a name too long must fail cleanly",
    )?;
    protected(computers, fixture).await
}

async fn planted<C: Computers>(computers: &C, fixture: &Confinement) -> Verdict {
    let here = fixture.workspace;
    for path in [
        "planted",
        "planted_dir/sentinel.txt",
        "planted_dir/made.txt",
    ] {
        ensure(
            matches!(
                computers.read(here, path).await,
                Err(ComputerError::Refused(_))
            ),
            &format!("reading through a planted link must be refused: {path}"),
        )?;
        let write = computers.write(here, &[plain(path, WRITTEN)]).await;
        ensure(
            matches!(write, Err(ComputerError::Refused(_))),
            &format!("writing through a planted link must be refused: {path}"),
        )?;
    }
    // A trailing slash must not stop a link to a file from being followed.
    for path in ["out_file/", "planted/", "out_file//"] {
        ensure(
            matches!(
                computers.read(here, path).await,
                Err(ComputerError::Refused(_))
            ),
            "a link to an outside file read with a trailing slash must be refused",
        )?;
        ensure(
            matches!(
                computers.write(here, &[plain(path, WRITTEN)]).await,
                Err(ComputerError::Refused(_))
            ),
            "a link to an outside file written with a trailing slash must be refused",
        )?;
    }
    outside_unchanged(computers, here, "a planted link let a write through:").await
}

/// A hard link shares the file itself, so no path check can tell it leads outside. Reading
/// it is fine (it is a file of the workspace); writing it replaces the workspace's name with
/// a new file, so the file outside keeps its content.
async fn hard_link<C: Computers>(computers: &C, fixture: &Confinement) -> Verdict {
    let here = fixture.workspace;
    let linked = computers.read(here, "hard.txt").await;
    ensure(
        linked.is_ok_and(|bytes| bytes.as_slice() == LINKED.as_bytes()),
        "a hard link reads as the file it is",
    )?;
    computers
        .write(here, &[plain("hard.txt", WRITTEN)])
        .await
        .map_err(failed("write a file that has another hard link"))?;
    let written = computers.read(here, "hard.txt").await;
    ensure(
        written.is_ok_and(|bytes| bytes.as_slice() == WRITTEN),
        "a hard-linked file reads back what was written",
    )?;
    outside_unchanged(computers, here, "a hard link let a write through:").await
}

/// Links that stay inside are followed, and answers name where they lead.
async fn links_inside<C: Computers>(computers: &C, fixture: &Confinement) -> Verdict {
    let here = fixture.workspace;
    computers
        .write(here, &[plain("in_link/through.txt", b"via")])
        .await
        .map_err(failed("write through a link to a folder inside"))?;
    for path in [
        "sub/through.txt",
        "self/self/in_link/through.txt",
        "in_link/../sub/through.txt",
    ] {
        let read = computers.read(here, path).await;
        ensure(
            read.is_ok_and(|bytes| bytes.as_slice() == b"via"),
            &format!("a link inside must be followed: {path}"),
        )?;
    }
    let stat = computers
        .stat(here, "in_link/through.txt")
        .await
        .map_err(failed("stat through a link inside"))?;
    ensure(
        stat.path == "sub/through.txt",
        "stat names where a link inside leads",
    )?;
    let listed = computers
        .list(here, &Listing::of("in_link"))
        .await
        .map_err(failed("list through a link inside"))?;
    ensure(
        listed
            .entries
            .iter()
            .any(|entry| entry.path == "sub/through.txt"),
        "a listing through a link inside shows what is there",
    )
}

/// A file replaced by a write keeps its permissions: a script stays executable.
async fn permissions<C: Computers>(computers: &C, fixture: &Confinement) -> Verdict {
    let here = fixture.workspace;
    run_quietly(
        computers,
        here,
        "printf old > run.sh && chmod 750 run.sh && printf old > private.txt && chmod 600 private.txt",
    )
    .await?;
    computers
        .write(
            here,
            &[plain("run.sh", b"new"), plain("private.txt", b"new")],
        )
        .await
        .map_err(failed("replace files with their own permissions"))?;
    let (_, modes) = run(
        computers,
        here,
        &Command::new("ls -l run.sh private.txt | cut -c1-10 | tr '\\n' ' '"),
    )
    .await?;
    ensure(
        modes.trim_end() == "-rw------- -rwxr-x---",
        &format!("a replaced file must keep its permissions: {modes:?}"),
    )
}

/// A link into a protected path resolves to that path, so the harness's check sees it; and a
/// write that asks to be protected is refused on the path it would really land on, links
/// followed, while one that does not ask, or lands elsewhere, goes through.
async fn protected<C: Computers>(computers: &C, fixture: &Confinement) -> Verdict {
    let here = fixture.workspace;
    let stat = computers
        .stat(here, "hooklink")
        .await
        .map_err(failed("stat a link into .glave"))?;
    ensure(
        stat.path == ".glave/hook.sh"
            && is_protected(
                std::path::Path::new(&fixture.root),
                &std::path::Path::new(&fixture.root).join(&stat.path),
            ),
        "a link into a protected path must resolve to it",
    )?;
    for (path, lands) in [
        ("hooklink", ".glave/hook.sh"),
        (".glave/new.json", ".glave/new.json"),
        ("in_link/../.glave/hook.sh", ".glave/hook.sh"),
        ("objects/pack/new.pack", "objects/pack/new.pack"),
    ] {
        let write = computers.write(here, &[guarded(path, GUARDED)]).await;
        ensure(
            write == Err(refuse_protected(lands)),
            &format!("a protected write to {path} must be refused as {lands}: {write:?}"),
        )?;
    }
    let hook = computers.read(here, ".glave/hook.sh").await;
    ensure(
        hook.is_ok_and(|bytes| bytes.as_slice() != GUARDED),
        "a refused protected write leaves the file as it was",
    )?;
    for nothing in [".glave/new.json", "objects"] {
        ensure(
            computers
                .stat(here, nothing)
                .await
                .is_ok_and(|stat| stat.kind.is_none()),
            &format!("a refused protected write makes nothing: {nothing}"),
        )?;
    }
    computers
        .write(here, &[guarded("sub/fine.txt", b"fine")])
        .await
        .map_err(failed("a protected write to an ordinary path"))?;
    computers
        .write(here, &[plain(".glave/unguarded.txt", b"fine")])
        .await
        .map_err(failed("an unprotected write to a protected path"))
}

/// What a protected write writes, which must never be found.
const GUARDED: &[u8] = b"GUARDED";

/// A write that asks the computer to refuse protected paths.
fn guarded(path: &str, content: &[u8]) -> FileWrite {
    FileWrite {
        protect: true,
        ..plain(path, content)
    }
}
