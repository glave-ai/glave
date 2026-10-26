//! What the lockfiles are compared with: the commit a change starts from.
//!
//! On CI, GitHub's event says, for a pull request (`pull_request.base.sha`, the base branch
//! GitHub merged it into) and for the merge queue (`merge_group.base_sha`, the commit its group
//! is built on). Elsewhere, and on CI for a push to main, it is where this branch left
//! origin/main, or, on main itself, the commit before this one (its first parent).
//!
//! A base older than the change's own start (a pull request's base from before main moved)
//! only adds versions to check, never hides one: what came into main since was checked as it
//! came, and has only aged. CI's checkout holds one commit, so a base it lacks is fetched, that
//! commit alone.

use serde_json::Value;

/// The commit the lockfiles are compared with, and what it is.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Base {
    commit: String,
    what: &'static str,
}

impl Base {
    /// What it is, and its commit, short, in words.
    pub(super) fn said(&self) -> String {
        let short = self.commit.get(..10).unwrap_or(&self.commit);
        format!("{} ({short})", self.what)
    }

    /// The text of `path`, from the workspace's root, at this commit, or `None` when the commit
    /// has no such file.
    pub(super) fn file(&self, path: &str) -> Result<Option<String>, String> {
        let listed = git(&["ls-tree", "--name-only", &self.commit, "--", path])?;
        if listed.trim().is_empty() {
            return Ok(None);
        }
        git(&["show", &format!("{}:{path}", self.commit)]).map(Some)
    }
}

/// The base of this run, fetched if this clone lacks it; `None` off CI with no origin/main to
/// compare with.
pub(super) fn find() -> Result<Option<Base>, String> {
    let base = match from_event()? {
        Some(base) => base,
        None => match from_git()? {
            Some(base) => base,
            None => return Ok(None),
        },
    };
    let commit = format!("{}^{{commit}}", base.commit);
    if git(&["rev-parse", "--quiet", "--verify", &commit]).is_err() {
        git(&[
            "fetch",
            "--quiet",
            "--no-tags",
            "--depth=1",
            "origin",
            &base.commit,
        ])
        .map_err(|err| {
            format!(
                "could not fetch {} to compare the lockfiles with: {err}",
                base.said()
            )
        })?;
    }
    Ok(Some(base))
}

/// The base GitHub's event names, on CI (`GITHUB_EVENT_NAME`, and the event itself at
/// `GITHUB_EVENT_PATH`).
fn from_event() -> Result<Option<Base>, String> {
    let (Ok(name), Some(path)) = (
        std::env::var("GITHUB_EVENT_NAME"),
        std::env::var_os("GITHUB_EVENT_PATH"),
    ) else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(&path).map_err(|err| {
        format!(
            "could not read GitHub's {name} event at {}: {err}",
            std::path::Path::new(&path).display()
        )
    })?;
    let event: Value = serde_json::from_str(&text)
        .map_err(|err| format!("GitHub's {name} event is not JSON: {err}"))?;
    in_event(&name, &event)
}

/// The base `event`, of the kind `name`, names: a pull request's or the merge queue's; `None`
/// for any other kind (a push), whose base comes from git.
fn in_event(name: &str, event: &Value) -> Result<Option<Base>, String> {
    let (pointer, what) = match name {
        "pull_request" => ("/pull_request/base/sha", "the pull request's base"),
        "merge_group" => ("/merge_group/base_sha", "the merge queue's base"),
        _ => return Ok(None),
    };
    let commit = event
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|commit| is_commit(commit))
        .ok_or_else(|| format!("GitHub's {name} event names no base commit at {pointer}"))?;
    Ok(Some(Base {
        commit: commit.to_owned(),
        what,
    }))
}

/// Where this branch left origin/main, or, on a commit main already has, the one before it;
/// on CI, with no origin/main, the one before this one; else `None`.
fn from_git() -> Result<Option<Base>, String> {
    let head = git(&["rev-parse", "HEAD"])?.trim().to_owned();
    let before = |what| -> Result<Option<Base>, String> {
        let object = git(&["cat-file", "commit", "HEAD"])?;
        let commit = first_parent(&object)
            .ok_or("this commit has no parent to compare the lockfiles with")?;
        Ok(Some(Base {
            commit: commit.to_owned(),
            what,
        }))
    };
    match git(&["merge-base", "HEAD", "origin/main"]) {
        Ok(base) if base.trim() != head => Ok(Some(Base {
            commit: base.trim().to_owned(),
            what: "where this branch left origin/main",
        })),
        Ok(_) => before("the commit before this one on main"),
        Err(_) if crate::parallel::on_ci() => before("the commit before this one"),
        Err(_) => Ok(None),
    }
}

/// The first parent a commit object, as `git cat-file commit` prints it, names. Read from the
/// object, as a shallow clone holds the commit but not its parents.
fn first_parent(commit: &str) -> Option<&str> {
    commit
        .lines()
        .take_while(|line| !line.is_empty())
        .find_map(|line| line.strip_prefix("parent "))
        .filter(|parent| is_commit(parent))
}

/// Whether `text` is a commit's full id: 40 hexadecimal digits, or 64 in a SHA-256 repository.
fn is_commit(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.chars().all(|c| c.is_ascii_hexdigit())
}

fn git(args: &[&str]) -> Result<String, String> {
    crate::cmd::query("git", args)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "3e68ede088b4e1866ed530d7461b769395e7f285";

    #[test]
    fn a_pull_request_and_the_merge_queue_name_their_base_and_a_push_does_not() {
        let pull = serde_json::json!({"pull_request": {"base": {"sha": BASE, "ref": "main"}}});
        assert_eq!(
            in_event("pull_request", &pull).unwrap().unwrap().said(),
            "the pull request's base (3e68ede088)"
        );
        let queue = serde_json::json!({"merge_group": {"base_sha": BASE, "head_sha": "x"}});
        let base = in_event("merge_group", &queue).unwrap().unwrap();
        assert_eq!(base.commit, BASE);
        assert_eq!(base.what, "the merge queue's base");
        let push = serde_json::json!({"before": BASE, "after": BASE});
        assert_eq!(in_event("push", &push), Ok(None));
        assert_eq!(in_event("workflow_dispatch", &push), Ok(None));
    }

    #[test]
    fn an_event_without_a_base_commit_is_refused() {
        let wrong = serde_json::json!({"pull_request": {"base": {"sha": "--upload-pack=x"}}});
        let err = in_event("pull_request", &wrong).unwrap_err();
        assert!(err.contains("names no base commit"), "{err}");
        assert!(in_event("merge_group", &serde_json::json!({})).is_err());
    }

    #[test]
    fn the_first_parent_is_read_from_the_commit_object() {
        let merge = format!(
            "tree 4f06629aa5597f0f3dc122b79d6f71005a16ef95\nparent {BASE}\n\
             parent 8ea2af5400000000000000000000000000000000\nauthor A <a@b> 1 +0000\n\n\
             parent in the message\n"
        );
        assert_eq!(first_parent(&merge), Some(BASE));
        let root = "tree 4f06629aa5597f0f3dc122b79d6f71005a16ef95\nauthor A <a@b> 1 +0000\n\n\
                    parent 3e68ede088b4e1866ed530d7461b769395e7f285\n";
        assert_eq!(first_parent(root), None);
    }
}
