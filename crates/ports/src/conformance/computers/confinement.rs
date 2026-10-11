//! Files stay inside their workspace, however the path is spelled and whatever links a
//! command planted. A fixture holds links to inside, outside, the parent, chains, loops and a
//! hard link, with a sentinel file next to the workspace. Every file operation on any path
//! must either stay inside or be refused, and the sentinel is never read or changed.

use super::helpers::{failed, run, run_quietly};
use super::{Verdict, ensure};
use crate::blob::digest;
use crate::computer::{
    Bytes, Command, ComputerError, ComputerId, Computers, Ending, FileWrite, Glob, Listing, Search,
    Sources, WorkspaceId, is_protected,
};

/// What the outside file holds; no answer may ever contain it.
const SENTINEL: &str = "SENTINEL";
/// What the hard-linked outside file holds; reading it through its link is allowed.
const LINKED: &str = "HARDLINK";

/// The links and files the fixture builds in the workspace, next to a folder outside it.
const BUILD: &str = "mkdir -p confine/inside/sub confine/inside/.glave confine/outside && \
    cd confine && printf SENTINEL > outside/sentinel.txt && printf HARDLINK > outside/linked.txt && \
    cd inside && printf inside > file.txt && printf hook > .glave/hook.sh &&printf nested > sub/deep.txt && \
    ln -s ../outside out_dir && ln -s ../outside/sentinel.txt out_file && \
    ln -s .. up && ln -s ../.. grand && ln -s . self && ln -s sub in_link && \
    ln -s l2 l1 && ln -s out_dir l2 && ln -s a b && ln -s b a && \
    ln -s ../outside/new.txt dangling && ln -s / rootlink && \
    ln -s ../../outside/sentinel.txt sub/deep_out && ln ../outside/linked.txt hard.txt && \
    ln -s .glave/hook.sh hooklink";

/// What the outside folder must hold after every operation.
const OUTSIDE_STATE: &str = "cd ../outside && ls -A | tr '\\n' ' ' && cat sentinel.txt linked.txt";
const OUTSIDE_UNTOUCHED: &str = "linked.txt sentinel.txt SENTINELHARDLINK";

/// Path pieces that make trouble: dot segments, empty ones, roots, links of every kind,
/// odd and long names, other systems' separators.
const PIECES: &[&str] = &[
    "..",
    ".",
    "",
    "/",
    "~",
    "out_dir",
    "out_file",
    "up",
    "grand",
    "self",
    "in_link",
    "l1",
    "l2",
    "a",
    "b",
    "dangling",
    "rootlink",
    "hard.txt",
    "file.txt",
    "sub",
    "deep.txt",
    "deep_out",
    "new",
    "new.txt",
    "ünï/çödé",
    "back\\slash",
    "..\\..",
    "sentinel.txt",
    "outside",
    "inside",
    "confine",
    "x/",
    "%2e%2e",
    ".git",
    ".glave",
    "hooklink",
    "\t",
    " ",
    "-rf",
    "*",
    "?",
    "\u{202e}",
    "\u{7f}",
    "a\nb",
];

/// The fixture: a workspace, and the absolute path of the folder outside it.
pub struct Confinement {
    workspace: WorkspaceId,
    root: String,
    long: String,
}

/// Builds the fixture inside the folder `here` has open, and opens its `inside` folder as a
/// workspace of its own.
pub async fn confinement_fixture<C: Computers>(
    computers: &C,
    computer: ComputerId,
    here: WorkspaceId,
    scratch: &str,
) -> Result<Confinement, String> {
    run_quietly(computers, here, BUILD).await?;
    let inside = computers
        .open(
            computer,
            &Sources::Folder {
                path: format!("{scratch}/confine/inside"),
            },
        )
        .await
        .map_err(failed("open the confinement fixture"))?;
    Ok(Confinement {
        workspace: inside.id,
        root: inside.root,
        long: "n".repeat(300),
    })
}

/// Paths from a fixed seed: `count` of them, the same on every run.
pub fn confinement_paths(seed: u64, count: usize) -> Vec<String> {
    let mut state = seed | 1;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    (0..count)
        .map(|_| {
            let length = 1 + usize::try_from(next() % 6).unwrap_or(1);
            let mut parts: Vec<&str> = Vec::new();
            for _ in 0..length {
                let at = usize::try_from(next()).unwrap_or(0) % PIECES.len();
                parts.push(PIECES.get(at).copied().unwrap_or(".."));
            }
            parts.join("/")
        })
        .collect()
}

/// Every operation on `path`: it stays inside, or is refused; the outside is never touched.
pub async fn confinement_case<C: Computers>(
    computers: &C,
    fixture: &Confinement,
    path: &str,
) -> Verdict {
    for candidate in [path.to_owned(), absolute_variant(fixture, path)] {
        one_path(computers, fixture, &candidate).await?;
    }
    Ok(())
}

/// The same path spelled from the outside folder's absolute name.
fn absolute_variant(fixture: &Confinement, path: &str) -> String {
    format!("{}/../outside/{path}", fixture.root)
}

fn acceptable<T>(result: &Result<T, ComputerError>) -> bool {
    !matches!(
        result,
        Err(ComputerError::Unreachable(_) | ComputerError::Unsupported(_))
    )
}

fn names_inside(path: &str) -> bool {
    !path.starts_with('/') && !path.split('/').any(|part| part == "..")
}

async fn one_path<C: Computers>(computers: &C, fixture: &Confinement, path: &str) -> Verdict {
    let here = fixture.workspace;
    let shown = |what: &str| format!("{what} {path:?}");
    let read = computers.read(here, path).await;
    ensure(
        acceptable(&read),
        &shown("reading must be answered or refused:"),
    )?;
    if let Ok(bytes) = read {
        ensure(
            !String::from_utf8_lossy(bytes.as_slice()).contains(SENTINEL),
            &shown("a read returned the outside file:"),
        )?;
    }
    let stat = computers.stat(here, path).await;
    ensure(
        acceptable(&stat),
        &shown("stat must be answered or refused:"),
    )?;
    if let Ok(stat) = stat {
        ensure(
            names_inside(&stat.path) && stat.digest != Some(digest(SENTINEL.as_bytes())),
            &shown("a stat named or hashed the outside:"),
        )?;
    }
    for (recursive, skip_ignored) in [(false, false), (true, false), (true, true)] {
        let listing = Listing {
            recursive,
            skip_ignored,
            ..Listing::of(path)
        };
        let listed = computers.list(here, &listing).await;
        ensure(
            acceptable(&listed),
            &shown("listing must be answered or refused:"),
        )?;
        if let Ok(listed) = listed {
            ensure(
                listed.entries.iter().all(|entry| names_inside(&entry.path)),
                &shown("a listing named the outside:"),
            )?;
        }
    }
    finding(computers, fixture, path).await?;
    let write = computers
        .write(
            here,
            &[FileWrite {
                path: path.to_owned(),
                content: Bytes::new(b"PWNED"),
            }],
        )
        .await;
    ensure(
        acceptable(&write),
        &shown("writing must be answered or refused:"),
    )?;
    // A refused write wrote nothing, so only the others need a look at the outside.
    if matches!(write, Err(ComputerError::Refused(_))) {
        return Ok(());
    }
    outside_untouched(computers, here, &shown("a write changed the outside:")).await
}

async fn finding<C: Computers>(computers: &C, fixture: &Confinement, path: &str) -> Verdict {
    let here = fixture.workspace;
    let glob = Glob {
        pattern: "*".into(),
        path: path.to_owned(),
        limit: 100,
    };
    let globbed = computers.glob(here, &glob).await;
    ensure(acceptable(&globbed), "a glob must be answered or refused")?;
    if let Ok(globbed) = globbed {
        ensure(
            names_inside(&globbed.path) && globbed.files.iter().all(|file| names_inside(file)),
            &format!("a glob named the outside: {path:?}"),
        )?;
    }
    let search = Search {
        pattern: ".".into(),
        path: path.to_owned(),
        glob: None,
        ignore_case: false,
        limit: 100,
        line_chars: 200,
    };
    let searched = computers.search(here, &search).await;
    ensure(
        acceptable(&searched),
        "a search must be answered or refused",
    )?;
    if let Ok(searched) = searched {
        ensure(
            searched
                .matches
                .iter()
                .all(|found| names_inside(&found.path) && !found.text.contains(SENTINEL)),
            &format!("a search read the outside: {path:?}"),
        )?;
    }
    Ok(())
}

/// The outside folder holds exactly what the fixture put there.
async fn outside_untouched<C: Computers>(
    computers: &C,
    here: WorkspaceId,
    violation: &str,
) -> Verdict {
    let (finished, output) = run(computers, here, &Command::new(OUTSIDE_STATE)).await?;
    ensure(
        finished.ending == (Ending::Exited { code: 0 }) && output.trim_end() == OUTSIDE_UNTOUCHED,
        &format!("{violation} {output:?}"),
    )
}

/// The targeted escapes: links a command plants, a hard link, a name too long, and links into
/// protected paths.
pub async fn confinement_escapes<C: Computers>(computers: &C, fixture: &Confinement) -> Verdict {
    let here = fixture.workspace;
    run_quietly(
        computers,
        here,
        "ln -s ../outside/sentinel.txt planted && ln -s ../outside planted_dir",
    )
    .await?;
    for path in [
        "planted",
        "planted_dir/sentinel.txt",
        "planted_dir/made.txt",
    ] {
        ensure(
            matches!(
                computers.read(here, path).await,
                Err(ComputerError::Refused(_))
            ) || path.ends_with("made.txt"),
            "reading through a planted link must be refused",
        )?;
        let write = computers.write(here, &[plain(path, b"PWNED")]).await;
        ensure(
            matches!(write, Err(ComputerError::Refused(_))),
            "writing through a planted link must be refused",
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
                computers.write(here, &[plain(path, b"PWNED")]).await,
                Err(ComputerError::Refused(_))
            ),
            "a link to an outside file written with a trailing slash must be refused",
        )?;
    }
    outside_untouched(computers, here, "a planted link let a write through:").await?;
    // A hard link shares the file itself, so no path check can tell: reading it is fine (it
    // is a file of the workspace), but writing it would change the outside file, so a file
    // with other hard links is never written.
    let linked = computers.read(here, "hard.txt").await;
    ensure(
        linked.is_ok_and(|bytes| bytes.as_slice() == LINKED.as_bytes()),
        "a hard link reads as the file it is",
    )?;
    ensure(
        matches!(
            computers.write(here, &[plain("hard.txt", b"PWNED")]).await,
            Err(ComputerError::Refused(_))
        ),
        "writing a file that has other hard links must be refused",
    )?;
    outside_untouched(computers, here, "a hard link let a write through:").await?;
    let long = computers.write(here, &[plain(&fixture.long, b"x")]).await;
    ensure(acceptable(&long), "a name too long must fail cleanly")?;
    protected(computers, fixture).await
}

/// A link into a protected path resolves to that path, so the harness's check sees it.
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
    )
}

fn plain(path: &str, content: &[u8]) -> FileWrite {
    FileWrite {
        path: path.to_owned(),
        content: Bytes::new(content),
    }
}
