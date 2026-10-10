//! Looking at files where they are, and the sandbox: what every computer must answer for the
//! harness's tools to behave the same on it.

use super::helpers::{Collect, Trip, failed, files};
use super::{Verdict, ensure};
use crate::blob::digest;
use crate::computer::{
    Command, ComputerError, Computers, EntryKind, Glob, Listing, Sandbox, SandboxLevel, Search,
    WorkspaceId,
};

const LIB: &[u8] = b"fn alpha() {}\nfn beta() {}\n";

pub(super) async fn finding<C: Computers>(computers: &C, here: WorkspaceId) -> Verdict {
    computers
        .write(
            here,
            &files(&[
                ("find/.gitignore", b"skip/\n"),
                ("find/src/lib.rs", LIB),
                ("find/skip/copy.rs", b"fn alpha() {}\n"),
                ("find/.env", b"alpha=1\n"),
                ("find/logo.bin", b"\0fn alpha"),
            ]),
        )
        .await
        .map_err(failed("write files to find"))?;
    stats(computers, here).await?;
    let listed = computers
        .list(
            here,
            &Listing {
                skip_ignored: true,
                ..Listing::of("find")
            },
        )
        .await
        .map_err(failed("list without ignored files"))?;
    let names: Vec<&str> = listed.entries.iter().map(|e| e.path.as_str()).collect();
    ensure(
        names == ["find/.env", "find/.gitignore", "find/logo.bin", "find/src"]
            && listed.ignored == 1,
        "a listing that skips ignored files leaves them out and counts them",
    )?;
    globs(computers, here).await?;
    searches(computers, here).await
}

async fn stats<C: Computers>(computers: &C, here: WorkspaceId) -> Verdict {
    let file = computers
        .stat(here, "find/src/../src/lib.rs")
        .await
        .map_err(failed("stat a file"))?;
    ensure(
        file.path == "find/src/lib.rs"
            && file.kind == Some(EntryKind::File)
            && file.size == LIB.len() as u64
            && file.digest == Some(digest(LIB)),
        "a stat resolves the path and gives the file's kind, size and digest",
    )?;
    let missing = computers
        .stat(here, "find/new/file.txt")
        .await
        .map_err(failed("stat nothing"))?;
    ensure(
        missing.path == "find/new/file.txt" && missing.kind.is_none(),
        "a stat of nothing resolves the path and finds nothing",
    )?;
    let folder = computers
        .stat(here, "find")
        .await
        .map_err(failed("stat a folder"))?;
    ensure(
        folder.kind == Some(EntryKind::Folder) && folder.digest.is_none(),
        "a stat of a folder says so",
    )?;
    ensure(
        matches!(
            computers.stat(here, "../outside").await,
            Err(ComputerError::Refused(_))
        ),
        "a stat outside the workspace must be refused",
    )
}

async fn globs<C: Computers>(computers: &C, here: WorkspaceId) -> Verdict {
    let glob = |pattern: &str| Glob {
        pattern: pattern.to_owned(),
        path: "find".into(),
        limit: 10,
    };
    let found = computers
        .glob(here, &glob("*.rs"))
        .await
        .map_err(failed("glob"))?;
    ensure(
        found.path == "find" && found.files == ["find/src/lib.rs"] && found.total == 1,
        "a glob finds matching files, leaving out what git ignores",
    )?;
    ensure(
        matches!(
            computers.glob(here, &glob("[")).await,
            Err(ComputerError::Refused(_))
        ),
        "an invalid file pattern must be refused",
    )
}

async fn searches<C: Computers>(computers: &C, here: WorkspaceId) -> Verdict {
    let search = |pattern: &str, limit: usize| Search {
        pattern: pattern.to_owned(),
        path: "find".into(),
        glob: None,
        ignore_case: false,
        limit,
        line_chars: 200,
    };
    let found = computers
        .search(here, &search("fn alpha", 10))
        .await
        .map_err(failed("search"))?;
    let lines: Vec<(&str, u64, &str)> = found
        .matches
        .iter()
        .map(|m| (m.path.as_str(), m.line, m.text.as_str()))
        .collect();
    ensure(
        lines == [("find/src/lib.rs", 1, "fn alpha() {}")]
            && found.secrets == ["find/.env"]
            && !found.more
            && !found.as_text,
        "a search finds lines, skipping ignored and binary files and naming secret ones",
    )?;
    let text = computers
        .search(here, &search("alpha(", 10))
        .await
        .map_err(failed("search for plain text"))?;
    ensure(
        text.as_text && text.matches.len() == 1,
        "a pattern that is not a regular expression is searched as text",
    )?;
    let capped = computers
        .search(here, &search("fn", 1))
        .await
        .map_err(failed("search with a limit"))?;
    ensure(
        capped.matches.len() == 1 && capped.more,
        "a search stops at its limit and says there is more",
    )
}

/// A sandboxed command is confined or refused, never run as if no sandbox was asked for.
pub(super) async fn sandboxed<C: Computers>(computers: &C, here: WorkspaceId) -> Verdict {
    let mut command = Command::new("echo inside > sandboxed.txt");
    command.sandbox = Some(Sandbox {
        level: SandboxLevel::ReadOnly,
        network: false,
        home: None,
        glave_home: None,
    });
    let output = Collect::new();
    let never = Trip::default();
    match computers.run(here, &command, &output, &never).await {
        Err(ComputerError::Unsupported(_)) => Ok(()),
        Err(error) => Err(format!("run a sandboxed command: {error}")),
        Ok(_) => ensure(
            matches!(
                computers.read(here, "sandboxed.txt").await,
                Err(ComputerError::NotFound(_))
            ),
            "a read-only sandbox must stop a command writing to the workspace",
        ),
    }
}
