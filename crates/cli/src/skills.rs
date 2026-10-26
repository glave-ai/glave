//! Skills on this computer: found in the project and in the person's folders, with glave's own
//! built in beside them, offered to the model as `skill`, and listed by `glave skills`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use glave_harness::{Found, Skill, SkillSource, Skills, find_skills, skill_folders};
use glave_ports::ToolSource;

use crate::home;

/// The person's own skill folders: `skills` in glave's folder, then `~/.agents/skills`, which
/// Codex and OpenCode read too.
fn personal() -> anyhow::Result<Vec<PathBuf>> {
    let mut folders = vec![home::folder()?.join("skills")];
    if let Some(home) = std::env::var_os("HOME") {
        folders.push(PathBuf::from(home).join(".agents/skills"));
    }
    Ok(folders)
}

/// The folders looked in for the project at `root`, first found first.
fn folders(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    Ok(skill_folders(root, &personal()?))
}

/// The skills for the project at `root`, with a warning printed for each one skipped.
pub(crate) async fn find(root: &Path) -> anyhow::Result<Found> {
    let found = with_built_in(find_skills(&folders(root)?).await);
    for warning in &found.warnings {
        eprintln!("glave: skill {warning}");
    }
    Ok(found)
}

/// `found` with glave's own skills after it.
fn with_built_in(mut found: Found) -> Found {
    for text in glave_office::SKILLS {
        match Skill::built_in(text) {
            Ok(skill) => found.skills.push(skill),
            Err(why) => found
                .warnings
                .push(format!("built into glave was skipped: {why}")),
        }
    }
    found
}

/// The `skill` tool for the project at `root`, when there are any skills; nothing otherwise.
pub(crate) async fn offered(root: &Path) -> anyhow::Result<Vec<Arc<dyn ToolSource>>> {
    let found = find(root).await?;
    Ok(Skills::offer(found.skills)
        .map(|skills| Arc::new(skills) as Arc<dyn ToolSource>)
        .into_iter()
        .collect())
}

/// `glave skills`: every skill the model is offered here, and where each comes from.
pub(crate) async fn list() -> anyhow::Result<ExitCode> {
    let root = std::env::current_dir()?;
    let found = find(&root).await?;
    for skill in &found.skills {
        let from = match &skill.source {
            SkillSource::File(file) => file.display().to_string(),
            SkillSource::BuiltIn(_) => "built into glave".to_owned(),
        };
        println!("{}\n  {}\n  {from}", skill.name, skill.description);
    }
    if !found
        .skills
        .iter()
        .any(|skill| matches!(skill.source, SkillSource::File(_)))
    {
        println!("No skills of yours or this project's here. glave looks for <name>/SKILL.md in:");
        for folder in folders(&root)? {
            println!("  {}", folder.display());
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glaves_own_skills_are_offered_beside_the_ones_found() {
        let found = with_built_in(Found::default());
        assert!(found.warnings.is_empty(), "{:?}", found.warnings);
        let names: Vec<&str> = found
            .skills
            .iter()
            .map(|skill| skill.name.as_str())
            .collect();
        assert_eq!(names, ["glave:spreadsheets"]);
    }
}
