//! Skills on a person's own computer: found there through the computer port, as they are in
//! folders here (`specs/computers.md`, "A person's computer in the cloud"), only when glave has
//! just used that computer, and kept with the instructions read there, so a thread opens with
//! them without reaching it.

use std::path::{Path, PathBuf};

use glave_event::SkillFound;
use glave_ports::computer::{ComputerError, EntryKind, Listing};

use super::{Gathered, PROJECT_FOLDERS, SKILL_MD, Skill, SkillSource, parsed};
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
    if !found.warnings.is_empty() {
        // Their paths and names are the person's, so not for the shared server's log.
        let skipped = found.warnings.len();
        tracing::warn!(skipped, "skills on a person's computer were skipped");
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
        // A file by that name, or a folder the computer won't show.
        Err(ComputerError::Refused(_)) => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut files = Vec::new();
    for entry in listed.entries {
        // A link counts, as a skill linked in does on a laptop; one to a file has none in it.
        if matches!(entry.kind, EntryKind::Folder | EntryKind::Link)
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
) -> Result<Result<Option<Skill>, String>, ComputerError> {
    let Some(path) = inside(home.root(), file) else {
        return Ok(Ok(None));
    };
    match home.read(&path).await {
        Ok(bytes) => Ok(parsed(&String::from_utf8_lossy(bytes.as_slice()), file)),
        Err(ComputerError::Refused(why)) => Ok(Err(why.to_string())),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use glave_computer::LocalComputers;
    use glave_ports::computer::{Computers as _, Environment, Sources};
    use glave_ports::testing::TempDir;

    use super::*;
    use crate::skills::{find_skills, skill_folders};

    fn skill(folder: &Path, name: &str, description: &str) {
        let dir = folder.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let text = format!("---\nname: {name}\ndescription: {description}\n---\nBody.\n");
        std::fs::write(dir.join(SKILL_MD), text).unwrap();
    }

    /// `base` as a person's home on a computer of this machine.
    async fn home_at(base: &Path) -> Workspace {
        let computers = LocalComputers::new(base.join(".copies"), std::env::vars_os());
        let made = computers.create(&Environment::host()).await.unwrap();
        let path = base.to_string_lossy().into_owned();
        let opened = computers
            .open(made.id, &Sources::Folder { path })
            .await
            .unwrap();
        Workspace::new(Arc::new(computers), opened)
    }

    #[test]
    fn what_is_found_there_is_what_a_laptop_finds_nearest_first() {
        let dir = TempDir::new("skills-on-home").unwrap();
        let base = std::fs::canonicalize(&dir).unwrap();
        let repository = base.join("glave/garden");
        let nested = repository.join("beds");
        std::fs::create_dir_all(repository.join(".git")).unwrap();
        std::fs::create_dir_all(&nested).unwrap();
        skill(&nested.join(".glave/skills"), "shared", "the nearest");
        skill(
            &repository.join(".glave/skills"),
            "shared",
            "the repository's",
        );
        skill(&repository.join(".agents/skills"), "repo", "another folder");
        let own = base.join(".glave/skills");
        skill(&own, "shared", "the person's");
        skill(&own, "own", "the person's own");
        // A file where a folder of skills would be is no failure, and a link counts.
        std::fs::write(repository.join(".claude"), "not a folder").unwrap();
        skill(&base.join("elsewhere"), "linked", "linked in");
        std::os::unix::fs::symlink(base.join("elsewhere/linked"), own.join("linked")).unwrap();
        glave_ports::block_on!(async {
            let home = home_at(&base).await;
            let found = skills_on(&home, Some(&nested)).await.unwrap();
            let personal = [own.clone(), base.join(".agents/skills")];
            let here = find_skills(&skill_folders(&nested, &personal)).await;
            let names = |pairs: Vec<(String, String)>| pairs;
            assert_eq!(
                names(found.into_iter().map(|s| (s.name, s.description)).collect()),
                names(
                    here.skills
                        .into_iter()
                        .map(|s| (s.name, s.description))
                        .collect()
                ),
            );
            let chats = skills_on(&home, None).await.unwrap();
            let chats: Vec<&str> = chats.iter().map(|s| s.name.as_str()).collect();
            assert_eq!(chats, ["linked", "own", "shared"]);
        });
    }

    #[test]
    fn a_home_gone_fails_the_read_so_nothing_half_read_is_kept() {
        let dir = TempDir::new("skills-home-gone").unwrap();
        let base = std::fs::canonicalize(&dir).unwrap().join("home");
        std::fs::create_dir_all(&base).unwrap();
        glave_ports::block_on!(async {
            let home = home_at(&base).await;
            std::fs::remove_dir_all(&base).unwrap();
            assert!(skills_on(&home, None).await.is_err());
        });
    }
}
