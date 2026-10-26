//! Files stay inside their workspace, however the path is spelled and whatever links a
//! command planted. A fixture holds links to inside, outside, the parent, chains, loops and a
//! hard link, with a sentinel file next to the workspace. Every file operation on any path
//! must either stay inside or be refused: no answer names or holds anything outside, a write
//! that succeeds lands inside (it reads back), and the folder around the workspace is
//! unchanged.

mod escapes;

pub use escapes::confinement_escapes;

use super::helpers::{failed, run, run_quietly};
use super::{Verdict, ensure};
use crate::blob::digest;
use crate::computer::{
    Bytes, Command, ComputerError, ComputerId, Computers, Ending, FileWrite, Glob, Listing, Search,
    Sources, WorkspaceId,
};

/// What the outside file holds; no answer may ever contain it.
const SENTINEL: &str = "SENTINEL";
/// What the hard-linked outside file holds; reading it through its link is allowed.
const LINKED: &str = "HARDLINK";
/// What every write in the suite writes.
const WRITTEN: &[u8] = b"PWNED";

/// The links and files the fixture builds in the workspace, next to a folder outside it.
const BUILD: &str = "mkdir -p confine/inside/sub confine/inside/.glave confine/outside && \
    cd confine && printf SENTINEL > outside/sentinel.txt && printf HARDLINK > outside/linked.txt && \
    cd inside && printf inside > file.txt && printf hook > .glave/hook.sh && printf nested > sub/deep.txt && \
    ln -s ../outside out_dir && ln -s ../outside/sentinel.txt out_file && \
    ln -s .. up && ln -s ../.. grand && ln -s . self && ln -s sub in_link && \
    ln -s l2 l1 && ln -s out_dir l2 && ln -s a b && ln -s b a && \
    ln -s ../outside/new.txt dangling && ln -s / rootlink && \
    ln -s ../../outside/sentinel.txt sub/deep_out && ln ../outside/linked.txt hard.txt && \
    ln -s .glave/hook.sh hooklink";

/// What is around the workspace (the `confine` folder), then the outside folder, then its two
/// files: all of it must be exactly as the fixture made it after every operation.
const OUTSIDE_STATE: &str = "cd .. && ls -A | tr '\\n' ' ' && ls -A outside | tr '\\n' ' ' && \
    cat outside/sentinel.txt outside/linked.txt";
const OUTSIDE_UNTOUCHED: &str = "inside outside linked.txt sentinel.txt SENTINELHARDLINK";

/// Every link and name of the fixture that leads somewhere interesting.
const LINKS: &[&str] = &[
    "out_dir",
    "out_file",
    "up",
    "grand",
    "self",
    "in_link",
    "l1",
    "l2",
    "a",
    "dangling",
    "rootlink",
    "sub/deep_out",
    "hard.txt",
    "hooklink",
    "file.txt",
    "sub",
    ".glave",
];

/// What follows a link in the fixed cases: itself, as a folder, the files it may hold, a way
/// back out.
const SUFFIXES: &[&str] = &[
    "",
    "/",
    "//",
    "/sentinel.txt",
    "/new.txt",
    "/../outside/sentinel.txt",
    "/confine/outside/sentinel.txt",
];

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

/// The fixture: a workspace, and the absolute path of its root.
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

/// The whole confinement suite: every link with every suffix, a seeded sample of odd paths,
/// what plainly inside must still work, and the targeted escapes.
pub(super) async fn confinement<C: Computers>(
    computers: &C,
    computer: ComputerId,
    here: WorkspaceId,
    scratch: &str,
) -> Verdict {
    let fixture = confinement_fixture(computers, computer, here, scratch).await?;
    confinement_untouched(computers, &fixture).await?;
    for path in confinement_targets()
        .into_iter()
        .chain(confinement_paths(0x5eed, 24))
    {
        confinement_case(computers, &fixture, &path).await?;
    }
    confinement_escapes(computers, &fixture).await
}

/// Every link of the fixture followed by every suffix, in a fixed order.
fn confinement_targets() -> Vec<String> {
    LINKS
        .iter()
        .flat_map(|link| SUFFIXES.iter().map(move |suffix| format!("{link}{suffix}")))
        .collect()
}

/// Paths from a fixed seed: `count` of them, the same on every run.
fn confinement_paths(seed: u64, count: usize) -> Vec<String> {
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
    for (recursive, skip_ignored) in [(false, false), (true, false), (true, true), (false, true)] {
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
    writing(computers, fixture, path).await
}

/// A write is refused, or lands inside: the path then resolves inside and reads back what was
/// written. Either way the outside is as it was.
async fn writing<C: Computers>(computers: &C, fixture: &Confinement, path: &str) -> Verdict {
    let here = fixture.workspace;
    let write = computers.write(here, &[plain(path, WRITTEN)]).await;
    ensure(
        acceptable(&write),
        &format!("writing must be answered or refused: {path:?}"),
    )?;
    // A refused write wrote nothing, so only the others need a look at the outside.
    if matches!(write, Err(ComputerError::Refused(_))) {
        return Ok(());
    }
    if write.is_ok() {
        let landed = computers.stat(here, path).await;
        ensure(
            landed
                .as_ref()
                .is_ok_and(|stat| names_inside(&stat.path) && stat.digest == Some(digest(WRITTEN))),
            &format!("a write that succeeded did not land inside: {path:?} {landed:?}"),
        )?;
    }
    let violation = format!("a write changed the outside: {path:?}");
    outside_unchanged(computers, here, &violation).await
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

/// Whether the folder around the workspace holds exactly what the fixture put there. Run on
/// its own, it shows the check itself works.
pub async fn confinement_untouched<C: Computers>(computers: &C, fixture: &Confinement) -> Verdict {
    outside_unchanged(computers, fixture.workspace, "the outside changed:").await
}

async fn outside_unchanged<C: Computers>(
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

fn plain(path: &str, content: &[u8]) -> FileWrite {
    FileWrite::new(path.to_owned(), Bytes::new(content))
}
