//! Skills on this computer: found in the project and in the person's folders, offered to the
//! model as `skill`, and listed by `glave skills`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use glave_harness::{Found, Skills, find_skills, skill_folders};
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
pub async fn find(root: &Path) -> anyhow::Result<Found> {
    let found = find_skills(&folders(root)?).await;
    for warning in &found.warnings {
        eprintln!("glave: skill {warning}");
    }
    Ok(found)
}

/// The `skill` tool for the project at `root`, when there are any skills; nothing otherwise.
pub async fn offered(root: &Path) -> anyhow::Result<Vec<Arc<dyn ToolSource>>> {
    let found = find(root).await?;
    Ok(Skills::offer(found.skills)
        .map(|skills| Arc::new(skills) as Arc<dyn ToolSource>)
        .into_iter()
        .collect())
}

/// `glave skills`: every skill the model is offered here, and where each comes from.
pub async fn list() -> anyhow::Result<ExitCode> {
    let root = std::env::current_dir()?;
    let found = find(&root).await?;
    if found.skills.is_empty() {
        println!("No skills here. glave looks for <name>/SKILL.md in:");
        for folder in folders(&root)? {
            println!("  {}", folder.display());
        }
        return Ok(ExitCode::SUCCESS);
    }
    for skill in &found.skills {
        println!(
            "{}\n  {}\n  {}",
            skill.name,
            skill.description,
            skill.file.display()
        );
    }
    Ok(ExitCode::SUCCESS)
}
