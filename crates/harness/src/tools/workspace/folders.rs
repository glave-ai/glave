//! Folders on a workspace's computer other than its own, as glave itself reaches them, never
//! the model: a folder looked at (where it resolves, and the repository it is the root of), the
//! folders inside one (for a person choosing a project's folder), and a new project's folder,
//! made empty as a git repository or cloned from one with the workspace's credentials.

use std::sync::Arc;

use glave_ports::{
    Command, ComputerError, Ending, EntryKind, Listing, Sources, WorkspaceId, WorkspaceInfo,
    origin_remote,
};

use super::{Collected, Workspace};
use crate::cancel::Cancel;

/// How long cloning a new project's repository may take.
const CLONE_SECS: u64 = 30 * 60;

/// A folder as glave found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Looked {
    /// Where it resolves, links followed: an absolute path on the computer.
    pub root: String,
    /// The `origin` remote of the repository it is the root of, if it is one.
    pub origin: Option<String>,
}

/// The folders inside one folder, by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inside {
    /// The folder listed, links followed.
    pub root: String,
    /// The names of the folders in it, in order, hidden ones (starting with `.`) left out.
    pub folders: Vec<String>,
    /// Whether there were more than one listing holds.
    pub truncated: bool,
}

impl Workspace {
    /// The folder at `path` on this workspace's computer: where it resolves, and its
    /// repository's `origin` remote when it is a repository's root. Not found when there is no
    /// folder there.
    pub async fn look(&self, path: &str) -> Result<Looked, ComputerError> {
        let info = self.open_plain(path).await?;
        let config = self.computers.read(info.id, ".git/config").await;
        self.forget(info.id).await;
        Ok(Looked {
            origin: config
                .ok()
                .and_then(|bytes| origin_remote(&String::from_utf8_lossy(bytes.as_slice()))),
            root: info.root,
        })
    }

    /// The folders inside the folder at `path` on this workspace's computer, by name: what a
    /// person picks a project's folder from. Not found when there is no folder there.
    pub async fn folders_in(&self, path: &str) -> Result<Inside, ComputerError> {
        let info = self.open_plain(path).await?;
        let listed = self.computers.list(info.id, &Listing::of(".")).await;
        self.forget(info.id).await;
        let listed = listed?;
        let mut folders: Vec<String> = listed
            .entries
            .into_iter()
            .filter(|entry| entry.kind == EntryKind::Folder && !entry.path.starts_with('.'))
            .map(|entry| entry.path)
            .collect();
        folders.sort_by_key(|name| name.to_lowercase());
        Ok(Inside {
            root: info.root,
            folders,
            truncated: listed.truncated,
        })
    }

    /// A new project's folder on this workspace's computer, named after `name` where the
    /// computer keeps projects: a new git repository, or a clone of `repository`, its git
    /// asking this workspace's credentials at each use, as a command's does. A folder whose
    /// clone failed is removed, and the error says what git said. Answers where it is.
    pub async fn new_project(
        &self,
        name: &str,
        repository: Option<&str>,
    ) -> Result<String, ComputerError> {
        let script = match repository {
            None => "git init --quiet".to_owned(),
            Some(url) => {
                let quoted = single_quoted(url).ok_or_else(|| {
                    ComputerError::Refused(format!("{url:?} is not a repository's address"))
                })?;
                format!("GIT_TERMINAL_PROMPT=0 git clone --quiet -- {quoted} .")
            }
        };
        let sources = Sources::Project {
            name: name.to_owned(),
        };
        let info = self.computers.open(self.info.computer, &sources).await?;
        let made = self.unconfined(&info, &script, CLONE_SECS).await;
        if made.is_err() {
            // Only what the failed clone left, in the folder just made for it.
            let _ = self
                .unconfined(&info, r#"cd .. && rm -rf -- "$OLDPWD""#, 60)
                .await;
        }
        self.forget(info.id).await;
        made.map(|()| info.root)
    }

    /// The folder at `path`, opened as it is: no sandbox, since glave alone reads it.
    async fn open_plain(&self, path: &str) -> Result<WorkspaceInfo, ComputerError> {
        let sources = Sources::Folder {
            path: path.to_owned(),
        };
        self.computers.open(self.info.computer, &sources).await
    }

    /// Lets go of a workspace opened only for a moment; the folder stays.
    async fn forget(&self, workspace: WorkspaceId) {
        let _ = self.computers.close(workspace).await;
    }

    /// Runs `script` for glave itself in `workspace`, outside the sandbox (it writes git's own
    /// files, which the sandbox protects), with this workspace's credentials when it has them.
    async fn unconfined(
        &self,
        workspace: &WorkspaceInfo,
        script: &str,
        timeout_secs: u64,
    ) -> Result<(), ComputerError> {
        let command = Command {
            timeout_secs: Some(timeout_secs),
            ..Command::new(script)
        };
        let output = Collected::default();
        let cancel = Cancel::new();
        let computers = Arc::clone(&self.computers);
        let finished = match self.credentials() {
            Some(credentials) => {
                computers
                    .run_with_credentials(
                        workspace.id,
                        &command,
                        &output,
                        &cancel,
                        credentials.as_ref(),
                    )
                    .await
            }
            None => {
                computers
                    .run(workspace.id, &command, &output, &cancel)
                    .await
            }
        }?;
        match finished.ending {
            Ending::Exited { code: 0 } => Ok(()),
            _ => Err(ComputerError::Failed(output.text().trim().to_owned())),
        }
    }
}

/// `text` in single quotes for bash, when it holds nothing a quote cannot carry: no quote of
/// its own, and no line break or other control character.
fn single_quoted(text: &str) -> Option<String> {
    (!text.is_empty() && !text.contains('\'') && !text.chars().any(char::is_control))
        .then(|| format!("'{text}'"))
}

#[cfg(test)]
mod tests {
    use super::single_quoted;

    #[test]
    fn only_what_a_quote_carries_is_quoted() {
        assert_eq!(
            single_quoted("https://example.com/a b.git").as_deref(),
            Some("'https://example.com/a b.git'")
        );
        for bad in ["", "it's", "a\nb", "a\u{7}b"] {
            assert_eq!(single_quoted(bad), None, "{bad:?}");
        }
    }
}
