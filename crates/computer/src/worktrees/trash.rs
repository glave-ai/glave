//! The person's Trash: where a worktree goes when its repository is gone, so nothing is ever
//! deleted outright. macOS keeps `~/.Trash`; Linux desktops keep the freedesktop.org Trash
//! (`files/` and an `info/<name>.trashinfo` saying where it came from).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use glave_ports::computer::ComputerError;

/// The most names tried before giving up on finding a free one in the Trash.
const MAX_NAMES: u32 = 1_000;

/// A Trash to move folders into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trash {
    /// A folder that things are moved into as they are: macOS's `~/.Trash`.
    Folder(PathBuf),
    /// A freedesktop.org Trash: things go in `files/`, with a note in `info/`.
    Freedesktop {
        root: PathBuf,
        /// When the move happens, as the note records it: `2026-10-10T12:00:00`.
        deleted_at: String,
    },
}

impl Trash {
    /// This machine's Trash for the person whose home folder is `home`, or `None` where there
    /// is none glave knows. `data_home` is `$XDG_DATA_HOME`, if set; `deleted_at` is now.
    pub fn for_home(home: &Path, data_home: Option<&Path>, deleted_at: &str) -> Option<Self> {
        if cfg!(target_os = "macos") {
            Some(Self::Folder(home.join(".Trash")))
        } else if cfg!(target_os = "linux") {
            let data = data_home.map_or_else(|| home.join(".local/share"), Path::to_path_buf);
            Some(Self::Freedesktop {
                root: data.join("Trash"),
                deleted_at: deleted_at.to_owned(),
            })
        } else {
            None
        }
    }

    /// The Trash of the person whose commands run with `environment`, as its `HOME` and
    /// `XDG_DATA_HOME` say; `None` with no `HOME`. `deleted_at` is now.
    pub(crate) fn for_environment(
        environment: &[(OsString, OsString)],
        deleted_at: &str,
    ) -> Option<Self> {
        let read = |name: &str| {
            environment
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| PathBuf::from(value))
        };
        Self::for_home(&read("HOME")?, read("XDG_DATA_HOME").as_deref(), deleted_at)
    }

    /// What it holds, at most `most`, by name, each with its note where the Trash keeps one
    /// (`info/<name>.trashinfo`); nothing when it can't be read.
    pub(crate) async fn held(&self, most: usize) -> Vec<(PathBuf, Option<PathBuf>)> {
        let (files, info) = match self {
            Self::Folder(folder) => (folder.clone(), None),
            Self::Freedesktop { root, .. } => (root.join("files"), Some(root.join("info"))),
        };
        let mut held = Vec::new();
        let Ok(mut read) = tokio::fs::read_dir(&files).await else {
            return held;
        };
        while held.len() < most {
            let Ok(Some(entry)) = read.next_entry().await else {
                break;
            };
            let note = info.as_ref().map(|info| {
                let mut name = entry.file_name();
                name.push(".trashinfo");
                info.join(name)
            });
            held.push((entry.path(), note));
        }
        held.sort();
        held
    }

    /// Moves the folder at `path` into the Trash under a free name, and answers where it is.
    /// A move that cannot be made in one step (another disk) fails and leaves it in place.
    pub async fn put(&self, path: &Path) -> Result<PathBuf, ComputerError> {
        let (files, info) = match self {
            Self::Folder(folder) => (folder.clone(), None),
            Self::Freedesktop { root, deleted_at } => {
                (root.join("files"), Some((root.join("info"), deleted_at)))
            }
        };
        let failed = |error: std::io::Error| {
            ComputerError::Failed(format!(
                "could not move {} to the Trash: {error}",
                path.display()
            ))
        };
        tokio::fs::create_dir_all(&files).await.map_err(failed)?;
        let name = path
            .file_name()
            .ok_or_else(|| {
                ComputerError::Refused(format!("{} has no name", path.display()).into())
            })?
            .to_string_lossy()
            .into_owned();
        for attempt in 0..MAX_NAMES {
            let free = if attempt == 0 {
                name.clone()
            } else {
                format!("{name} {}", attempt.saturating_add(1))
            };
            let dest = files.join(&free);
            if tokio::fs::symlink_metadata(&dest).await.is_ok() {
                continue;
            }
            if let Some((info, deleted_at)) = &info {
                tokio::fs::create_dir_all(info).await.map_err(failed)?;
                let note = format!(
                    "[Trash Info]\nPath={}\nDeletionDate={deleted_at}\n",
                    path.display()
                );
                // Made only where none is, as the Trash's other users make theirs: a name
                // taken meanwhile moves on to the next.
                let at = info.join(format!("{free}.trashinfo"));
                let made = tokio::task::spawn_blocking(move || {
                    glave_ports::file::create(&at, note.as_bytes())
                })
                .await
                .map_err(|error| failed(std::io::Error::other(error)))?;
                match made {
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    made => made.map_err(failed)?,
                }
            }
            if let Err(error) = tokio::fs::rename(path, &dest).await {
                // A note on nothing would date what isn't there.
                if let Some((info, _)) = &info {
                    let _gone =
                        tokio::fs::remove_file(info.join(format!("{free}.trashinfo"))).await;
                }
                return Err(failed(error));
            }
            return Ok(dest);
        }
        Err(ComputerError::Failed(format!(
            "no free name in the Trash for {}",
            path.display()
        )))
    }

    /// Moves `trashed`, something [`Trash::put`] left directly in this Trash, back to `to`,
    /// which must not exist, and drops its note: whether it came back. Nothing else in the
    /// Trash, and nothing outside it, is ever moved.
    pub(crate) async fn take_back(&self, trashed: &Path, to: &Path) -> bool {
        let (files, info) = match self {
            Self::Folder(folder) => (folder.clone(), None),
            Self::Freedesktop { root, .. } => (root.join("files"), Some(root.join("info"))),
        };
        let Some(name) = trashed.file_name() else {
            return false;
        };
        if trashed.parent() != Some(files.as_path())
            || tokio::fs::symlink_metadata(to).await.is_ok()
            || tokio::fs::rename(trashed, to).await.is_err()
        {
            return false;
        }
        if let Some(info) = info {
            let mut note = name.to_os_string();
            note.push(".trashinfo");
            let _gone = tokio::fs::remove_file(info.join(note)).await;
        }
        true
    }
}
