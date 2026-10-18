//! The person's Trash: where a worktree goes when its repository is gone, so nothing is ever
//! deleted outright. macOS keeps `~/.Trash`; Linux desktops keep the freedesktop.org Trash
//! (`files/` and an `info/<name>.trashinfo` saying where it came from).

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
            .ok_or_else(|| ComputerError::Refused(format!("{} has no name", path.display())))?
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
                tokio::fs::write(info.join(format!("{free}.trashinfo")), note)
                    .await
                    .map_err(failed)?;
            }
            tokio::fs::rename(path, &dest).await.map_err(failed)?;
            return Ok(dest);
        }
        Err(ComputerError::Failed(format!(
            "no free name in the Trash for {}",
            path.display()
        )))
    }
}
