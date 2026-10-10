//! Skills, in the open Agent Skills format (agentskills.io): a folder holding a SKILL.md, whose
//! frontmatter gives a `name` and a `description`, and whose body tells the model how to do one
//! kind of task, with any scripts and files beside it. The model sees only names and
//! descriptions until a task needs one; then `skill` loads its body. Scripts a skill mentions
//! run only through bash, asked about like any other command.

pub(crate) mod frontmatter;
mod tool;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use frontmatter::Value;

pub use tool::{SKILL, Skills};

/// The file that makes a folder a skill.
pub const SKILL_MD: &str = "SKILL.md";

/// The folders in a project, and in the person's home, that hold skills: glave's own, then the
/// shared `.agents/skills` that Codex and OpenCode read, then Claude Code's.
pub const PROJECT_FOLDERS: [&str; 3] = [".glave/skills", ".agents/skills", ".claude/skills"];

/// The most skills offered, so a crowded folder cannot crowd the model's context.
pub const MAX_SKILLS: usize = 100;

/// The most characters of names and descriptions offered in all, as Codex budgets its listing
/// when it does not know the context window.
pub const MAX_LISTED: usize = 8_000;

/// The longest name, and the longest description, the format allows.
const MAX_NAME: usize = 64;
const MAX_DESCRIPTION: usize = 1024;

/// A skill the model may load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// Its SKILL.md.
    pub file: PathBuf,
}

impl Skill {
    /// The skill's folder, which its body's paths are relative to.
    pub fn folder(&self) -> &Path {
        self.file.parent().unwrap_or(&self.file)
    }

    /// How much of the listing this skill takes.
    fn listed(&self) -> usize {
        self.name
            .chars()
            .count()
            .saturating_add(self.description.chars().count())
    }
}

/// The skills found, by name, and what was skipped or hidden, each with the file and why.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    pub skills: Vec<Skill>,
    pub warnings: Vec<String>,
}

/// The folders skills are looked for in, first found first: in each folder from `root` up to
/// its repository's root (only `root` outside a repository), the project folders; then
/// `personal`, the person's own. A name is kept by the first folder that has it, so the
/// project nearest the work wins, as a nested AGENTS.md does.
pub fn skill_folders(root: &Path, personal: &[PathBuf]) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = crate::instructions::project_dirs(root)
        .iter()
        .rev()
        .flat_map(|dir| PROJECT_FOLDERS.map(|folder| dir.join(folder)))
        .collect();
    folders.extend_from_slice(personal);
    folders
}

/// Every valid skill in `folders`, each a subfolder holding a SKILL.md, sorted by name. A
/// skill that cannot be read or breaks the format is skipped with a warning, never fatal, and
/// past [`MAX_SKILLS`] or [`MAX_LISTED`] the rest are left out with one.
pub async fn find_skills(folders: &[PathBuf]) -> Found {
    let mut found = Found::default();
    let mut names: HashMap<String, PathBuf> = HashMap::new();
    let mut listed = 0usize;
    let mut left_out = Vec::new();
    for folder in folders {
        for file in skill_files(folder, &mut found.warnings).await {
            let skill = match read(&file).await {
                Ok(Some(skill)) => skill,
                Ok(None) => continue,
                Err(why) => {
                    found
                        .warnings
                        .push(format!("skipped {}: {why}", file.display()));
                    continue;
                }
            };
            if let Some(kept) = names.get(&skill.name) {
                found.warnings.push(format!(
                    "skipped {}: the skill {:?} in {} comes first",
                    file.display(),
                    skill.name,
                    kept_folder(kept)
                ));
                continue;
            }
            names.insert(skill.name.clone(), skill.file.clone());
            let after = listed.saturating_add(skill.listed());
            if found.skills.len() >= MAX_SKILLS || after > MAX_LISTED {
                left_out.push(skill.name);
                continue;
            }
            listed = after;
            found.skills.push(skill);
        }
    }
    if !left_out.is_empty() {
        found.warnings.push(format!(
            "left out {} (at most {MAX_SKILLS} skills and {MAX_LISTED} characters of names \
             and descriptions are offered)",
            left_out.join(", ")
        ));
    }
    found.skills.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/// The SKILL.md in each subfolder of `folder`, in name order. A folder that is not there has
/// none; one that cannot be read is a warning.
async fn skill_files(folder: &Path, warnings: &mut Vec<String>) -> Vec<PathBuf> {
    let mut entries = match tokio::fs::read_dir(folder).await {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(err) if err.kind() == std::io::ErrorKind::NotADirectory => return Vec::new(),
        Err(err) => {
            warnings.push(format!("could not read {}: {err}", folder.display()));
            return Vec::new();
        }
    };
    let mut files = Vec::new();
    loop {
        match entries.next_entry().await {
            Ok(Some(entry)) => {
                let file = entry.path().join(SKILL_MD);
                if tokio::fs::metadata(&file)
                    .await
                    .is_ok_and(|meta| meta.is_file())
                {
                    files.push(file);
                }
            }
            Ok(None) => break,
            Err(err) => {
                warnings.push(format!("could not read {}: {err}", folder.display()));
                break;
            }
        }
    }
    files.sort();
    files
}

/// The folder of the skill a SKILL.md path belongs to, for a warning.
fn kept_folder(file: &Path) -> String {
    file.parent().unwrap_or(file).display().to_string()
}

/// The skill in `file`: `None` when it asks not to be offered to the model
/// (`disable-model-invocation: true`, as Claude Code writes it), or why it is not valid.
async fn read(file: &Path) -> Result<Option<Skill>, String> {
    let text = tokio::fs::read_to_string(file)
        .await
        .map_err(|err| err.to_string())?;
    let document = frontmatter::parse(&text)?;
    let text_field = |key: &str| match document.fields.get(key) {
        Some(Value::Text(text)) => Ok(Some(text.trim().to_owned())),
        Some(Value::Other) => Err(format!("{key} must be text")),
        None => Ok(None),
    };
    if text_field("disable-model-invocation")?.as_deref() == Some("true") {
        return Ok(None);
    }
    let folder = file
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    // Without a name, the folder names it, as Claude Code and Codex read it.
    let name = text_field("name")?.unwrap_or_else(|| folder.to_owned());
    check_name(&name, folder)?;
    let description = text_field("description")?.ok_or("its frontmatter has no description")?;
    check_description(&description)?;
    Ok(Some(Skill {
        name,
        description,
        file: file.to_path_buf(),
    }))
}

/// A name is 1 to 64 lowercase letters, digits and single hyphens, neither first nor last,
/// and matches its folder's name.
fn check_name(name: &str, folder: &str) -> Result<(), String> {
    let allowed = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
    if name.is_empty() || name.chars().count() > MAX_NAME {
        return Err(format!("its name must be 1 to {MAX_NAME} characters"));
    }
    if !name.chars().all(allowed) {
        return Err(format!(
            "its name {name:?} may hold only lowercase letters, digits and hyphens"
        ));
    }
    if name.starts_with('-') || name.ends_with('-') || name.contains("--") {
        return Err(format!(
            "its name {name:?} may not start or end with a hyphen or hold two in a row"
        ));
    }
    if name != folder {
        return Err(format!(
            "its name {name:?} must match its folder's name, {folder:?}"
        ));
    }
    Ok(())
}

/// A description says what the skill does and when to use it, in 1 to 1024 characters.
fn check_description(description: &str) -> Result<(), String> {
    if description.is_empty() {
        return Err("its description is empty".to_owned());
    }
    let length = description.chars().count();
    if length > MAX_DESCRIPTION {
        return Err(format!(
            "its description is {length} characters; at most {MAX_DESCRIPTION} are allowed"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
