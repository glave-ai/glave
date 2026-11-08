//! What changed in a folder since a point in its history: every file that differs from it,
//! whatever changed it (a command here), new and removed ones among them, a binary one named
//! without its lines, an ignored one left out, and what was committed since still counted. A
//! folder that keeps no history says so.

use super::helpers::{failed, run_quietly};
use super::{Verdict, ensure};
use crate::computer::{ChangeKind, ChangedFile, ComputerId, Computers, LineKind, Sources};

/// Files committed in `seed`, the repository `moves_through_git` leaves.
const COMMITTED: &str = "cd seed && printf 'one\\ntwo\\n' > kept.txt && printf 'x\\n' > gone.txt \
    && printf 'ignored.txt\\n' > .gitignore && git add -A && \
    git -c user.name=seed -c user.email=seed@example.com commit -q -m files";

/// What a command changes there afterwards, one change committed.
const CHANGED: &str = "cd seed && printf 'one\\nTWO\\n' > kept.txt && rm gone.txt && \
    printf 'a\\nb\\n' > new.txt && printf 'x\\000y' > blob.bin && printf 'no\\n' > ignored.txt \
    && printf 'c\\n' > committed.txt && git add committed.txt && \
    git -c user.name=seed -c user.email=seed@example.com commit -q -m more";

/// Runs after `worktrees`, in the folder `root`, where `seed` is, as the computer names it;
/// `scratch` holds `plain`, a folder in no repository.
pub(super) async fn changes<C: Computers>(
    computers: &C,
    computer: ComputerId,
    here: crate::computer::WorkspaceId,
    (root, scratch): (&str, &str),
) -> Verdict {
    run_quietly(computers, here, COMMITTED).await?;
    let folder = |path: String| Sources::Folder { path };
    let seed = computers
        .open(computer, &folder(format!("{root}/seed")))
        .await
        .map_err(failed("open a repository's folder"))?;
    let at = seed.at.clone();
    ensure(
        at.as_ref().is_some_and(|at| at.len() >= 40),
        "a folder in a repository opens where its history stands",
    )?;
    let unchanged = computers.changes(seed.id, at.as_deref()).await;
    ensure(
        unchanged.is_ok_and(|changes| changes.is_some_and(|changes| changes.files.is_empty())),
        "a folder as it opened has nothing changed",
    )?;
    run_quietly(computers, here, CHANGED).await?;
    let since = computers
        .changes(seed.id, at.as_deref())
        .await
        .map_err(failed("read what changed"))?
        .ok_or("a repository's folder keeps history")?;
    let paths: Vec<&str> = since.files.iter().map(|file| file.path.as_str()).collect();
    ensure(
        paths
            == [
                "blob.bin",
                "committed.txt",
                "gone.txt",
                "kept.txt",
                "new.txt",
            ],
        "every file that differs, by path, new and removed among them, the ignored one not",
    )?;
    ensure(
        since.shortened
            && since
                .files
                .first()
                .is_some_and(|blob| blob.changes.is_empty()),
        "a binary file is named without its lines",
    )?;
    shown(&since.files)?;
    let now = computers
        .changes(seed.id, None)
        .await
        .map_err(failed("read what changed since now"))?;
    let named = |changes: Option<crate::computer::Changes>| {
        changes.map(|changes| changes.files.into_iter().map(|file| file.path).collect())
    };
    let now: Option<Vec<String>> = named(now);
    ensure(
        now.as_ref().is_some_and(|now| now.len() == 4),
        "since where it stands now, what was committed is no change",
    )?;
    let odd = computers.changes(seed.id, Some("--output=x")).await;
    ensure(
        odd.is_ok_and(|odd| named(odd) == now),
        "what names no commit is read as now, never as an option",
    )?;
    let plain = computers
        .open(computer, &folder(format!("{scratch}/plain")))
        .await
        .map_err(failed("open a plain folder"))?;
    let none = computers.changes(plain.id, None).await;
    ensure(
        plain.at.is_none() && none.is_ok_and(|none| none.is_none()),
        "a folder in no repository keeps no history, and says so",
    )?;
    for workspace in [seed.id, plain.id] {
        computers
            .close(workspace)
            .await
            .map_err(failed("close a folder"))?;
    }
    Ok(())
}

/// Checks the lines of the files a command changed: a stretch of an edited file, the whole of
/// a new one, the lines of a removed one taken out.
fn shown(files: &[ChangedFile]) -> Verdict {
    let expected = |added, removed, lines: &[&str]| {
        let lines = lines.iter().map(|line| (*line).to_owned());
        Some((added, removed, lines.collect::<Vec<_>>()))
    };
    ensure(
        signed(files, "kept.txt") == expected(1, 1, &["edited", " one", "-two", "+TWO"]),
        "an edited file shows its stretch, with the lines kept around it",
    )?;
    ensure(
        signed(files, "new.txt") == expected(2, 0, &["wrote", "+a", "+b"]),
        "a new file shows whole",
    )?;
    ensure(
        signed(files, "gone.txt") == expected(0, 1, &["edited", "-x"]),
        "a removed file shows its lines taken out",
    )
}

/// The file at `path` among `files`: its counts, and each change as its kind, then its lines,
/// each after its sign.
fn signed(files: &[ChangedFile], path: &str) -> Option<(u64, u64, Vec<String>)> {
    let file = files.iter().find(|file| file.path == path)?;
    let lines = file.changes.iter().flat_map(|change| {
        let kind = match change.kind {
            ChangeKind::Wrote => "wrote",
            ChangeKind::Edited | ChangeKind::EditedEvery => "edited",
        };
        let lines = change.lines.iter().map(|line| {
            let sign = match line.kind {
                LineKind::Same => ' ',
                LineKind::Removed => '-',
                LineKind::Added => '+',
            };
            format!("{sign}{}", line.text)
        });
        std::iter::once(kind.to_owned()).chain(lines)
    });
    Some((file.added, file.removed, lines.collect()))
}
