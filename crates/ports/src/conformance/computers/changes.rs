//! What changed in a folder since a point in its history: every file that differs from it,
//! whatever changed it (a command here), new and removed ones among them, a binary one named
//! without its lines, an ignored one and a repository inside left out, and what was committed
//! since still counted, read without writing in the repository; which point it was read since,
//! where it stands now when the one asked for is gone; a repository with no commit yet standing
//! at nothing. A folder that keeps no history, or that its repository ignores, says so.

use super::helpers::{failed, run, run_quietly};
use super::{Verdict, ensure};
use crate::computer::{
    ChangeKind, ChangedFile, Changes, Command, ComputerId, Computers, LineKind, Since, Sources,
    WorkspaceId,
};

/// Files committed in `seed`, the repository `moves_through_git` leaves.
const COMMITTED: &str = "cd seed && printf 'one\\ntwo\\n' > kept.txt && printf 'x\\n' > gone.txt \
    && printf 'ignored.txt\\nskip/\\n' > .gitignore && git add -A && \
    git -c user.name=seed -c user.email=seed@example.com commit -q -m files";

/// What a command changes there afterwards, one change committed, beside a repository of its
/// own with no commit and a folder the repository ignores.
const CHANGED: &str = "cd seed && printf 'one\\nTWO\\n' > kept.txt && rm gone.txt && \
    printf 'a\\nb\\n' > new.txt && printf 'x\\000y' > blob.bin && printf 'no\\n' > ignored.txt \
    && mkdir -p skip inner && printf 's\\n' > skip/s.txt && git -C inner init -q && \
    printf 'i\\n' > inner/i.txt && printf 'c\\n' > committed.txt && git add committed.txt && \
    git -c user.name=seed -c user.email=seed@example.com commit -q -m more";

/// What tells whether `seed`'s index or objects changed.
const LOOK: &str = "cd seed && cksum .git/index && find .git/objects -type f | wc -l";

/// Runs after `worktrees`, in the folder `root`, where `seed` is, as the computer names it;
/// `scratch` holds `plain`, a folder in no repository.
pub(super) async fn changes<C: Computers>(
    computers: &C,
    computer: ComputerId,
    here: WorkspaceId,
    (root, scratch): (&str, &str),
) -> Verdict {
    run_quietly(computers, here, COMMITTED).await?;
    let seed = open(computers, computer, &format!("{root}/seed")).await?;
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
    let (_, before) = run(computers, here, &Command::new(LOOK)).await?;
    let since = read(computers, seed.id, at.as_deref()).await?;
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
        "every file that differs, by path, new and removed among them, the ignored ones and a \
         repository inside not",
    )?;
    ensure(
        since.since == Since::Asked
            && since.shortened
            && since
                .files
                .first()
                .is_some_and(|blob| blob.changes.is_empty()),
        "read since the point asked, a binary file named without its lines",
    )?;
    shown(&since.files)?;
    let now = read(computers, seed.id, None).await?;
    ensure(
        now.since == Since::Now && now.files.len() == 4,
        "since where it stands now, what was committed is no change",
    )?;
    let gone = "0123456789abcdef0123456789abcdef01234567";
    for point in [gone, "--output=x"] {
        let odd = read(computers, seed.id, Some(point)).await?;
        ensure(
            odd.since == Since::Now && odd.files == now.files,
            "a point the history lacks, or one no commit could be, is read as now, never as an \
             option",
        )?;
    }
    let (_, after) = run(computers, here, &Command::new(LOOK)).await?;
    ensure(
        !before.is_empty() && before == after,
        "reading what changed writes nothing in the repository: its index and objects as they were",
    )?;
    computers
        .close(seed.id)
        .await
        .map_err(failed("close a folder"))?;
    elsewhere(computers, computer, here, (root, scratch)).await
}

/// A repository with no commit yet, a folder its repository ignores, and one in none.
async fn elsewhere<C: Computers>(
    computers: &C,
    computer: ComputerId,
    here: WorkspaceId,
    (root, scratch): (&str, &str),
) -> Verdict {
    run_quietly(
        computers,
        here,
        "git init -q fresh && printf 'n\\n' > fresh/n.txt",
    )
    .await?;
    let fresh = open(computers, computer, &format!("{root}/fresh")).await?;
    let new = read(computers, fresh.id, fresh.at.as_deref()).await?;
    ensure(
        fresh.at.is_some()
            && new.since == Since::Asked
            && new
                .files
                .iter()
                .map(|file| file.path.as_str())
                .eq(["n.txt"]),
        "a repository with no commit yet stands at nothing, so all in it is new",
    )?;
    let mut folders = vec![fresh.id];
    for (path, why) in [
        (
            format!("{root}/seed/skip"),
            "a folder its repository ignores",
        ),
        (format!("{scratch}/plain"), "a folder in no repository"),
    ] {
        let folder = open(computers, computer, &path).await?;
        folders.push(folder.id);
        let none = computers.changes(folder.id, None).await;
        ensure(none.is_ok_and(|none| none.is_none()), why)?;
    }
    for workspace in folders {
        computers
            .close(workspace)
            .await
            .map_err(failed("close a folder"))?;
    }
    Ok(())
}

/// The folder at `path`, opened in place.
async fn open<C: Computers>(
    computers: &C,
    computer: ComputerId,
    path: &str,
) -> Result<crate::computer::WorkspaceInfo, String> {
    let sources = Sources::Folder {
        path: path.to_owned(),
    };
    let opened = computers.open(computer, &sources).await;
    opened.map_err(failed("open a folder"))
}

/// What changed in `workspace` since `since`, which must keep history.
async fn read<C: Computers>(
    computers: &C,
    workspace: WorkspaceId,
    since: Option<&str>,
) -> Result<Changes, String> {
    let read = computers.changes(workspace, since).await;
    read.map_err(failed("read what changed"))?
        .ok_or_else(|| "a repository's folder keeps history".to_owned())
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
