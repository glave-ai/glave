//! A thread's scratch folder: a folder of its own for a thread with no project, such as a
//! one-off chat, under the scratch base, with its owner kept beside it as a worktree's is. Kept
//! until a cleanup removes it; opening the same name again reopens it as it was left.

use std::path::{Path, PathBuf};

use glave_ports::computer::{ComputerError, Refusal};

use super::paths;
use super::worktree::{Record, write_record};

/// The longest scratch folder name.
const MAX_NAME: usize = 64;

/// The scratch folder `name` under `base`, made if it is not there yet, with `owner` kept
/// beside it and any hold cleared.
pub(crate) async fn open(
    base: &Path,
    name: &str,
    owner: Option<&str>,
) -> Result<PathBuf, ComputerError> {
    if name.is_empty()
        || name.len() > MAX_NAME
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(ComputerError::Refused(
            Refusal::ScratchName {
                name: format!("{name:?}"),
                most: MAX_NAME.to_string(),
            }
            .into(),
        ));
    }
    let dest = base.join(name);
    tokio::fs::create_dir_all(&dest).await.map_err(|error| {
        ComputerError::Failed(format!("could not make {}: {error}", dest.display()))
    })?;
    let record = Record {
        owner: owner.map(str::to_owned),
        ..Record::default()
    };
    write_record(&dest, &record).await?;
    paths::folder(&dest.to_string_lossy()).await
}
