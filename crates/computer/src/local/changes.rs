//! What changed in a workspace's folder since a point in its history (`files/changes`), as
//! git tells it: every file that differs from a commit, whatever changed it, the new files git
//! doesn't ignore among them, read without writing anything in the person's repository. New
//! files are marked in a copy of the index, in a folder of glave's own ([`apart`]), so git
//! compares them as any other; its answer is read up to a limit and drawn as changes
//! ([`patch`]). glave's own git, outside any sandbox, so run as `git.rs` runs every git: no
//! hooks, no file-system monitor, no program a repository names; here no external diff or text
//! conversion either, and never inside a repository the folder holds, whose own config could
//! name a program.

mod apart;
mod patch;

use std::ffi::OsString;
use std::path::Path;

use glave_ports::computer::{Changes, ComputerError, Since};

use super::git::{Git, LEAVE_SUBMODULES};
use super::registry::Opened;
use apart::Apart;

/// The most of git's answer read, in bytes, and of its list of new files: past it, files are
/// named with their counts and without their lines, and new files past it go unnamed.
const MOST_READ: usize = 1024 * 1024;

/// The largest file git reads the lines of: a larger one it counts as binary, named without
/// them, rather than read whole.
const LARGEST_READ: &str = "core.bigFileThreshold=8m";

/// What git says of a folder in no repository, in the C locale.
const NO_REPOSITORY: &str = "not a git repository";

/// Where the folder of `opened` stands in its history now: its commit, when it is in a
/// repository that has one; the empty tree in one with no commit yet, so all it gets is new.
pub(crate) async fn at(opened: &Opened) -> Option<String> {
    let git = Git::at(&opened.processes, &opened.environment, &opened.root);
    let head = git.ask(&["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]);
    if let Ok(Ok(commit)) = head.await {
        return Some(commit).filter(|commit| is_point(commit));
    }
    let inside = git.ask(&["rev-parse", "--is-inside-work-tree"]).await;
    if !matches!(&inside, Ok(Ok(inside)) if inside == "true") {
        return None;
    }
    let empty = git.must(&["hash-object", "-t", "tree", "/dev/null"]).await;
    empty.ok().filter(|tree| is_point(tree))
}

/// What changed in the folder of `opened` since the point `since`, or since where it stands
/// now when that is `None` or gone, working in a folder of glave's own in its computer's folder
/// in `base`; `None` for a folder in no repository, or one its repository ignores. A repository
/// git can't read (owned by someone else, damaged) is an error, never no history.
pub(crate) async fn since(
    opened: &Opened,
    since: Option<&str>,
    base: &Path,
) -> Result<Option<Changes>, ComputerError> {
    let in_c: Vec<(OsString, OsString)> = opened
        .environment
        .iter()
        .cloned()
        .chain([("LC_ALL".into(), "C".into())])
        .collect();
    let git = Git::at(&opened.processes, &in_c, &opened.root);
    let found = git
        .ask(&[
            "rev-parse",
            "--is-inside-work-tree",
            "--show-prefix",
            "--path-format=absolute",
            "--git-path",
            "index",
            "--git-path",
            "objects",
        ])
        .await?;
    let found = match found {
        Ok(found) => found,
        Err(said) if said.contains(NO_REPOSITORY) => return Ok(None),
        Err(said) => return Err(ComputerError::Failed(said)),
    };
    let mut found = found.lines();
    let (Some(inside), Some(prefix), Some(index), Some(objects)) =
        (found.next(), found.next(), found.next(), found.next())
    else {
        return Err(ComputerError::Failed(format!(
            "git rev-parse told too little about {}",
            opened.root.display()
        )));
    };
    // The repository's own folder, or one it ignores, keeps no history of its own.
    let ignored = !prefix.is_empty() && git.ask(&["check-ignore", "--quiet", "."]).await?.is_ok();
    if inside != "true" || ignored {
        return Ok(None);
    }
    let (from, since) = from(&git, since).await?;
    // In the computer's own folder, which goes with it.
    let own = base.join(opened.computer.to_string());
    let apart = Apart::made(&own, Path::new(index)).await?;
    let environment = apart.environment(&opened.environment, objects);
    let git = Git {
        options: vec!["-c".into(), LARGEST_READ.into()],
        ..Git::at(&opened.processes, &environment, &opened.root)
    };
    let unmarked = mark_new(&git, &apart).await?;
    let printed = git
        .printed(
            &[
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                LEAVE_SUBMODULES,
                // Nor `diff.submodule` set to `log` or `diff`, which would run git there.
                "--submodule=short",
                "--no-renames",
                "--relative",
                "-O/dev/null",
                "--src-prefix=a/",
                "--dst-prefix=b/",
                "--unified=3",
                "--numstat",
                "--patch",
                "-z",
                &from,
                "--",
            ],
            MOST_READ,
        )
        .await?;
    let (printed, cut) = printed.map_err(ComputerError::Failed)?;
    let mut changes = patch::changes(&printed, cut, since);
    changes.shortened |= unmarked;
    Ok(Some(changes))
}

/// Marks each new file git doesn't ignore in the copy of the index `apart` holds as one meant
/// to be added, so the diff counts it: never a repository inside the folder, which git lists as
/// a folder, and would take in for one it tracks, then look inside with that repository's own
/// config. Whether some new files went unmarked, past the most read.
async fn mark_new(git: &Git<'_>, apart: &Apart) -> Result<bool, ComputerError> {
    let listed = git.printed(
        &[
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            ".",
        ],
        MOST_READ,
    );
    let (listed, cut) = listed.await?.map_err(ComputerError::Failed)?;
    let mut new: Vec<&[u8]> = listed
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty() && !path.ends_with(b"/"))
        .collect();
    if cut {
        // Its last one may be cut short.
        new.pop();
    }
    if new.is_empty() {
        return Ok(cut);
    }
    let paths = apart.write("new", &new.join(&0)).await?;
    let from_file = format!("--pathspec-from-file={}", paths.display());
    let marking = ["add", "--intent-to-add", &from_file, "--pathspec-file-nul"];
    git.must(&marking).await?;
    Ok(cut)
}

/// The tree changes are read since, and which point it is: `since` while the repository still
/// has it, else where it stands now, else, in a repository with no commit yet, the empty tree.
async fn from(git: &Git<'_>, since: Option<&str>) -> Result<(String, Since), ComputerError> {
    let asked = since
        .filter(|since| is_point(since))
        .map(|since| (since, Since::Asked));
    for (point, which) in asked.into_iter().chain([("HEAD", Since::Now)]) {
        let tree = format!("{point}^{{tree}}");
        if let Ok(tree) = git
            .ask(&["rev-parse", "--verify", "--quiet", &tree])
            .await?
        {
            return Ok((tree, which));
        }
    }
    let empty = git
        .must(&["hash-object", "-t", "tree", "/dev/null"])
        .await?;
    Ok((empty, Since::Now))
}

/// Whether `text` names a commit or a tree as git writes one: 40 hex digits, or 64 in a
/// repository of SHA-256, so it can never be read as an option or a name.
fn is_point(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}
