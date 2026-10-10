//! Custom commands on this computer: found in the project and in the person's folders, sent
//! when the person types `/name arguments`, and listed by `glave commands`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use glave_harness::{Command, FoundCommands, command_folders, find_commands};

use crate::home;

/// What the interactive session does itself, so no command may take the name.
const BUILT_IN: [&str; 2] = ["exit", "compact"];

/// The person's own command folders: `commands` in glave's folder, then Claude Code's.
fn personal() -> anyhow::Result<Vec<PathBuf>> {
    let mut folders = vec![home::folder()?.join("commands")];
    if let Some(home) = std::env::var_os("HOME") {
        folders.push(PathBuf::from(home).join(".claude/commands"));
    }
    Ok(folders)
}

/// The folders looked in for the project at `root`, first found first.
fn folders(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    Ok(command_folders(root, &personal()?))
}

/// The commands for the project at `root`, with a warning printed for each one skipped.
pub async fn find(root: &Path) -> anyhow::Result<Vec<Command>> {
    let FoundCommands {
        mut commands,
        mut warnings,
    } = find_commands(&folders(root)?).await;
    commands.retain(|command| {
        let built_in = BUILT_IN.contains(&command.name.as_str());
        if built_in {
            warnings.push(format!(
                "skipped {}: /{} is built in",
                command.file.display(),
                command.name
            ));
        }
        !built_in
    });
    for warning in &warnings {
        eprintln!("glave: command {warning}");
    }
    Ok(commands)
}

/// `glave commands`: every command that can be typed here, and where each comes from.
pub async fn list() -> anyhow::Result<ExitCode> {
    let root = std::env::current_dir()?;
    let commands = find(&root).await?;
    if commands.is_empty() {
        println!("No commands here. glave looks for <name>.md in:");
        for folder in folders(&root)? {
            println!("  {}", folder.display());
        }
        return Ok(ExitCode::SUCCESS);
    }
    for command in &commands {
        let hint = command
            .argument_hint
            .as_deref()
            .map(|hint| format!(" {hint}"))
            .unwrap_or_default();
        println!("/{}{hint}", command.name);
        if let Some(description) = &command.description {
            println!("  {description}");
        }
        println!("  {}", command.file.display());
    }
    Ok(ExitCode::SUCCESS)
}
