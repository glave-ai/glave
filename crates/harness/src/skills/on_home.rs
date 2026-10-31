//! Skills on a person's own computer: found there through the computer port, as they are in
//! folders here (`specs/computers.md`, "A person's computer in the cloud"), only when glave has
//! just used that computer, and kept with the instructions read there, so a thread opens with
//! them without reaching it.

use std::path::{Path, PathBuf};

use glave_event::SkillFound;
use glave_ports::computer::{ComputerError, EntryKind, Listing};

use super::{Gathered, PROJECT_FOLDERS, SKILL_MD, SkillSource, parsed};
use crate::instructions::{dirs_on, inside, there};
use crate::tools::workspace::Workspace;

/// What a thread in the folder at `root` is offered from its person's computer, through `home`,
/// their home on it: the project's skills from `root` up to its repository's root, nearest
/// first, then their own. For a chat, with no `root`, their own alone. A skill that can't be read
/// is left out; the computer or its home gone fails the read, so nothing half read is kept.
pub(crate) async fn skills_on(
    home: &Workspace,
    root: Option<&Path>,
) -> Result<Vec<SkillFound>, ComputerError> {
    let mut folders: Vec<PathBuf> = Vec::new();
    if let Some(root) = root {
        for dir in dirs_on(home, root).await?.iter().rev() {
            folders.extend(PROJECT_FOLDERS.map(|folder| dir.join(folder)));
        }
    }
    folders.extend([".glave/skills", ".agents/skills"].map(|folder| home.root().join(folder)));
    let mut gathered = Gathered::default();
    for folder in &folders {
        for file in files_in(home, folder).await? {
            let read = read_on(home, &file).await?;
            gathered.add(&file, read);
        }
    }
    let found = gathered.finish();
    for warning in &found.warnings {
        tracing::warn!(%warning, "a skill on a person's computer");
    }
    Ok(found
        .skills
        .into_iter()
        .filter_map(|skill| match skill.source {
            SkillSource::File(file) => Some(SkillFound {
                name: skill.name,
                description: skill.description,
                path: file.display().to_string(),
            }),
            SkillSource::BuiltIn(_) => None,
        })
        .collect())
}

/// The SKILL.md of each folder in `folder` there, in name order.
async fn files_in(home: &Workspace, folder: &Path) -> Result<Vec<PathBuf>, ComputerError> {
    let Some(path) = inside(home.root(), folder) else {
        return Ok(Vec::new());
    };
    if !there(home, &path).await? {
        return Ok(Vec::new());
    }
    let listed = match home.list(&Listing::of(path)).await {
        Ok(listed) => listed,
        // Not a folder, or one the computer won't show.
        Err(ComputerError::Refused(_)) => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut files = Vec::new();
    for entry in listed.entries {
        if entry.kind == EntryKind::Folder
            && there(home, &format!("{}/{SKILL_MD}", entry.path)).await?
        {
            files.push(home.root().join(entry.path).join(SKILL_MD));
        }
    }
    Ok(files)
}

/// The skill in the SKILL.md at `file` there: why not, for one that can't be read.
async fn read_on(
    home: &Workspace,
    file: &Path,
) -> Result<Result<Option<super::Skill>, String>, ComputerError> {
    let Some(path) = inside(home.root(), file) else {
        return Ok(Ok(None));
    };
    match home.read(&path).await {
        Ok(bytes) => Ok(parsed(&String::from_utf8_lossy(bytes.as_slice()), file)),
        Err(ComputerError::Refused(why)) => Ok(Err(why.to_string())),
        Err(error) => Err(error),
    }
}
