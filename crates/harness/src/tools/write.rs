//! `write`: create a file, or replace one the model has read.

use serde::Deserialize;

use super::ToolOutcome;
use super::workspace::Workspace;

#[derive(Debug, Deserialize)]
pub(crate) struct WriteArgs {
    path: String,
    content: String,
}

pub(crate) async fn write(workspace: &Workspace, args: WriteArgs) -> ToolOutcome {
    let path = match workspace.resolve_for_change(&args.path).await {
        Ok(path) => path,
        Err(message) => return ToolOutcome::error(message),
    };
    let shown = workspace.show(&path);
    let existed = match tokio::fs::metadata(&path).await {
        Ok(metadata) if metadata.is_dir() => {
            return ToolOutcome::error(format!("{shown} is a folder, not a file."));
        }
        Ok(_) => true,
        Err(_) => false,
    };
    if existed {
        let current = match tokio::fs::read(&path).await {
            Ok(current) => current,
            Err(e) => return ToolOutcome::error(format!("Could not read {shown}: {e}")),
        };
        if let Err(message) = workspace.check_fresh(&path, &current) {
            return ToolOutcome::error(format!(
                "Refusing to overwrite {shown}: {message} To change part of it, use edit."
            ));
        }
    }
    if let Some(parent) = path.parent()
        && let Err(e) = tokio::fs::create_dir_all(parent).await
    {
        return ToolOutcome::error(format!("Could not create the folder for {shown}: {e}"));
    }
    if let Err(e) = tokio::fs::write(&path, args.content.as_bytes()).await {
        return ToolOutcome::error(format!("Could not write {shown}: {e}"));
    }
    workspace.remember(&path, args.content.as_bytes());
    let lines = args.content.lines().count();
    let kind = if existed {
        "replaced the existing file"
    } else {
        "new file"
    };
    ToolOutcome::ok(format!("Wrote {lines} lines to {shown} ({kind})."))
}
