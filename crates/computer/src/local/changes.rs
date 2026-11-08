//! What changed in a workspace's folder since a point in its history (`files/changes`), as
//! git tells it: every file that differs from a commit, whatever changed it, the new files git
//! doesn't ignore among them, read without writing anything in the person's repository. New
//! files are marked in a copy of the index, whose objects go to a folder of glave's own beside
//! it, so git compares them as any other; its answer is read up to a limit and drawn as changes
//! ([`patch`]). glave's own git, outside any sandbox, so run as `git.rs` runs every git: no
//! hooks, no file-system monitor, no program a repository names, and here no external diff or
//! text conversion either.

mod patch;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use glave_ports::computer::{Changes, ComputerError};

use super::git::Git;
use super::registry::{Opened, new_id};

/// The most of git's answer read, in bytes: past it, files are named with their counts and
/// without their lines.
const MOST_READ: usize = 1024 * 1024;

/// The largest file git reads the lines of: a larger one it counts as binary, named without
/// them, rather than read whole.
const LARGEST_READ: &str = "core.bigFileThreshold=8m";

/// Where the folder of `opened` stands in its history now: its commit, when it is in a
/// repository that has one.
pub(crate) async fn at(opened: &Opened) -> Option<String> {
    let git = Git::at(&opened.processes, &opened.environment, &opened.root);
    let head = git.ask(&["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]);
    head.await.ok()?.ok().filter(|commit| is_commit(commit))
}

/// What changed in the folder of `opened` since the commit `since`, or since where it stands
/// now when that is `None` or gone; `None` when it is in no repository.
pub(crate) async fn since(
    opened: &Opened,
    since: Option<&str>,
) -> Result<Option<Changes>, ComputerError> {
    let git = Git::at(&opened.processes, &opened.environment, &opened.root);
    let found = git
        .ask(&[
            "rev-parse",
            "--is-inside-work-tree",
            "--path-format=absolute",
            "--git-path",
            "index",
            "--git-path",
            "objects",
        ])
        .await?;
    let Ok(found) = found else {
        return Ok(None);
    };
    let mut found = found.lines();
    let (Some("true"), Some(index), Some(objects)) = (found.next(), found.next(), found.next())
    else {
        return Ok(None);
    };
    let from = from(&git, since).await?;
    let apart = Apart::made(Path::new(index)).await?;
    let environment = apart.environment(&opened.environment, objects);
    let git = Git {
        options: vec!["-c".into(), LARGEST_READ.into()],
        ..Git::at(&opened.processes, &environment, &opened.root)
    };
    if let Err(said) = git.ask(&["add", "--intent-to-add", "--", "."]).await?
        && git.ask(&["check-ignore", "--quiet", "."]).await?.is_err()
    {
        // Unless the folder itself is ignored, so nothing new in it would count.
        return Err(ComputerError::Failed(said));
    }
    let printed = git
        .printed(
            &[
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
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
    Ok(Some(patch::changes(&printed, cut)))
}

/// The commit changes are read since: `since` while the repository still has it, else where
/// it stands now, else, in a repository with no commit yet, the empty tree.
async fn from(git: &Git<'_>, since: Option<&str>) -> Result<String, ComputerError> {
    let known = since.filter(|since| is_commit(since)).into_iter();
    for point in known.chain(["HEAD"]) {
        let commit = format!("{point}^{{commit}}");
        if let Ok(commit) = git
            .ask(&["rev-parse", "--verify", "--quiet", &commit])
            .await?
        {
            return Ok(commit);
        }
    }
    git.must(&["hash-object", "-t", "tree", "/dev/null"]).await
}

/// Whether `text` names a commit as git writes one: 40 hex digits, or 64 in a repository of
/// SHA-256, so it can never be read as an option or a name.
fn is_commit(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// A folder of glave's own for one read, removed with it: a copy of the repository's index,
/// which new files are marked in, and the objects git makes meanwhile.
struct Apart(PathBuf);

impl Apart {
    /// The folder, with a copy of the index at `index` when the repository has one yet.
    async fn made(index: &Path) -> Result<Self, ComputerError> {
        let apart = Self(std::env::temp_dir().join(format!("glave-changes-{}", new_id()?)));
        let failed = |error: std::io::Error| {
            ComputerError::Failed(format!("could not make {}: {error}", apart.0.display()))
        };
        tokio::fs::create_dir_all(apart.0.join("objects"))
            .await
            .map_err(failed)?;
        match tokio::fs::copy(index, apart.0.join("index")).await {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(failed(error)),
            _ => Ok(apart),
        }
    }

    /// `environment`, with git's index and new objects here, and the repository's `objects`
    /// read beside them.
    fn environment(
        &self,
        environment: &[(OsString, OsString)],
        objects: &str,
    ) -> Vec<(OsString, OsString)> {
        let quoted = objects.replace('\\', "\\\\").replace('"', "\\\"");
        let apart: [(OsString, OsString); 4] = [
            ("GIT_INDEX_FILE".into(), self.0.join("index").into()),
            ("GIT_OBJECT_DIRECTORY".into(), self.0.join("objects").into()),
            (
                "GIT_ALTERNATE_OBJECT_DIRECTORIES".into(),
                format!("\"{quoted}\"").into(),
            ),
            // Nor does git take the index's lock to refresh it.
            ("GIT_OPTIONAL_LOCKS".into(), "0".into()),
        ];
        environment.iter().cloned().chain(apart).collect()
    }
}

impl Drop for Apart {
    fn drop(&mut self) {
        // A copy of an index and an empty object, removed in moments.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
