//! `write`: create a file, or replace one the model has read.

use glave_ports::EntryKind;
use serde::Deserialize;

use super::ToolOutcome;
use super::workspace::Workspace;

#[derive(Debug, Deserialize)]
pub(crate) struct WriteArgs {
    path: String,
    content: String,
}

pub(crate) async fn write(workspace: &Workspace, args: WriteArgs) -> ToolOutcome {
    let stat = match workspace.stat_for_change(&args.path).await {
        Ok(stat) => stat,
        Err(message) => return ToolOutcome::error(message),
    };
    let shown = stat.path;
    let existed = match stat.kind {
        Some(EntryKind::Folder) => {
            return ToolOutcome::error(format!("{shown} is a folder, not a file."));
        }
        Some(EntryKind::File | EntryKind::Link) => true,
        None => false,
    };
    if existed && let Err(message) = workspace.check_fresh(&shown, stat.digest.as_ref()) {
        return ToolOutcome::error(format!(
            "Refusing to overwrite {shown}: {message} To change part of it, use edit."
        ));
    }
    if let Err(message) = workspace.write(&shown, args.content.as_bytes()).await {
        return ToolOutcome::error(message);
    }
    workspace.remember(&shown, args.content.as_bytes());
    let lines = args.content.lines().count();
    let kind = if existed {
        "replaced the existing file"
    } else {
        "new file"
    };
    ToolOutcome::ok(format!("Wrote {lines} lines to {shown} ({kind})."))
}
