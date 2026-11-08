//! A folder of glave's own for one read of what changed, under the computer's own folder and
//! open to its owner alone: a copy of the repository's index, which new files are marked in, the
//! list of them, and the objects git makes meanwhile, so nothing is written in the person's
//! repository. Removed as the read ends; one a killed glave left is removed by a later read.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use glave_ports::computer::ComputerError;
use glave_ports::{Clock as _, SystemClock};

use crate::local::registry::new_id;

/// How long a folder left by a read is kept: far past the longest a read takes, a few of git's
/// reads one after another.
const LEFT_FOR: Duration = Duration::from_hours(1);

/// The folder, removed when it drops.
pub(super) struct Apart(PathBuf);

impl Apart {
    /// A new one in `base`'s `changes` folder, its owner's alone, with a copy of the index at
    /// `index` when the repository has one yet; any a read left there an hour ago or more
    /// removed first.
    pub(super) async fn made(base: &Path, index: &Path) -> Result<Self, ComputerError> {
        let parent = base.join("changes");
        reap(&parent).await;
        let apart = Self(parent.join(new_id()?.to_string()));
        let failed = |error: std::io::Error| {
            ComputerError::Failed(format!("could not make {}: {error}", apart.0.display()))
        };
        let mut folders = tokio::fs::DirBuilder::new();
        folders.recursive(true).mode(0o700);
        folders
            .create(apart.0.join("objects"))
            .await
            .map_err(failed)?;
        match tokio::fs::copy(index, apart.0.join("index")).await {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(failed(error)),
            _ => Ok(apart),
        }
    }

    /// Writes `content` to the file `name` here, and says where it is.
    pub(super) async fn write(&self, name: &str, content: &[u8]) -> Result<PathBuf, ComputerError> {
        let path = self.0.join(name);
        tokio::fs::write(&path, content).await.map_err(|error| {
            ComputerError::Failed(format!("could not write {}: {error}", path.display()))
        })?;
        Ok(path)
    }

    /// `environment`, with git's index and new objects here, the repository's `objects` read
    /// beside them, and every path it is given taken as it is written, never as a pattern.
    pub(super) fn environment(
        &self,
        environment: &[(OsString, OsString)],
        objects: &str,
    ) -> Vec<(OsString, OsString)> {
        let quoted = objects.replace('\\', "\\\\").replace('"', "\\\"");
        let apart: [(OsString, OsString); 5] = [
            ("GIT_INDEX_FILE".into(), self.0.join("index").into()),
            ("GIT_OBJECT_DIRECTORY".into(), self.0.join("objects").into()),
            (
                "GIT_ALTERNATE_OBJECT_DIRECTORIES".into(),
                format!("\"{quoted}\"").into(),
            ),
            ("GIT_LITERAL_PATHSPECS".into(), "1".into()),
            // Nor does git take the index's lock to refresh it.
            ("GIT_OPTIONAL_LOCKS".into(), "0".into()),
        ];
        environment.iter().cloned().chain(apart).collect()
    }
}

impl Drop for Apart {
    fn drop(&mut self) {
        // A copy of an index, a list and an empty object, removed in moments.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Removes from `parent` each folder a read left there `LEFT_FOR` ago or more, its glave killed
/// before it could; one that can't be told or removed stays for the next.
async fn reap(parent: &Path) {
    let Ok(mut entries) = tokio::fs::read_dir(parent).await else {
        return;
    };
    let now = SystemTime::from(SystemClock.now());
    while let Ok(Some(entry)) = entries.next_entry().await {
        let left = entry.metadata().await.and_then(|data| data.modified());
        if left.is_ok_and(|left| now.duration_since(left).is_ok_and(|age| age >= LEFT_FOR)) {
            let _ = tokio::fs::remove_dir_all(entry.path()).await;
        }
    }
}
