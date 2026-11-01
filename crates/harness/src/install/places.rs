//! Where `install` puts what it adds: the project's files, through its computer, beside its other
//! changes (`.agents/skills`, which Codex and OpenCode read too, and `.mcp.json`); or the
//! person's own, in glave's folder here (`skills` and `mcp.json`), which glave reads again before
//! each turn. Every other server in a `.mcp.json` stays as it was written.

use std::path::{Path, PathBuf};

use glave_event::ServiceOption;
use glave_ports::computer::ComputerError;
use glave_ports::settings::mcp::Edited;
use serde_json::value::RawValue;

use super::{Change, Own};
use crate::skills::SKILL_MD;
use crate::tools::workspace::Workspace;

/// A project's skills, as `install` adds them.
const PROJECT_SKILLS: &str = ".agents/skills";
/// A project's tool servers.
const PROJECT_SERVERS: &str = ".mcp.json";
/// The person's own skills and tool servers, in glave's folder.
const OWN_SKILLS: &str = "skills";
const OWN_SERVERS: &str = "mcp.json";

/// Where something goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Place {
    /// The project's files: anyone who gets the project gets it too.
    Project,
    /// The person's own: just theirs, in all their projects.
    Everywhere,
}

impl Place {
    /// The choice the person is shown.
    pub(crate) fn option(self) -> ServiceOption {
        let (value, label) = match self {
            Self::Project => (
                "project",
                "This project: saved in this project's files, so anyone who gets the project \
                 gets it too",
            ),
            Self::Everywhere => (
                "everywhere",
                "Everywhere: just for you, in all your projects",
            ),
        };
        ServiceOption {
            value: value.to_owned(),
            label: label.to_owned(),
        }
    }

    /// The place a choice's value names.
    pub(crate) fn from_value(value: &str) -> Option<Self> {
        [Self::Project, Self::Everywhere]
            .into_iter()
            .find(|place| place.option().value == value)
    }
}

/// Whether `change`'s skill or server is in `place` now.
pub(crate) async fn holds(
    workspace: &Workspace,
    own: Option<&Own>,
    place: Place,
    change: &Change,
) -> Result<bool, String> {
    let name = change.name();
    match (place, change.is_skill()) {
        (Place::Project, true) => {
            let stat = workspace.stat(&project_skill(name)).await?;
            Ok(stat.kind.is_some())
        }
        (Place::Project, false) => {
            let edited = Edited::read(&project_servers(workspace).await?).map_err(unreadable)?;
            Ok(edited.servers.get(name).is_some())
        }
        (Place::Everywhere, skill) => {
            let Some(own) = own else {
                return Ok(false);
            };
            let (folder, name) = (own.folder.clone(), name.to_owned());
            blocking(move || {
                if skill {
                    return Ok(own_skill(&folder, &name).exists());
                }
                let edited = Edited::read(&own_servers(&folder)?).map_err(unreadable)?;
                Ok(edited.servers.get(&name).is_some())
            })
            .await
        }
    }
}

/// Makes `change` in `place`: what the model is told it came to, or why it could not.
pub(crate) async fn apply(
    workspace: &Workspace,
    own: Option<&Own>,
    place: Place,
    change: &Change,
) -> Result<String, String> {
    let replaces = change.adds() && holds(workspace, own, place, change).await?;
    match place {
        Place::Project => in_project(workspace, change).await?,
        Place::Everywhere => {
            let own = own.ok_or("The person has no folder of their own here.")?;
            let (folder, change) = (own.folder.clone(), change.clone());
            blocking(move || in_own(&folder, &change)).await?;
        }
    }
    Ok(told(place, change, replaces))
}

/// What the model is told came of `change` in `place`.
fn told(place: Place, change: &Change, replaced: bool) -> String {
    let name = change.name();
    let replacing = if replaced {
        " It replaces the one by that name."
    } else {
        ""
    };
    let said = match (place, change) {
        (Place::Project, Change::AddSkill { .. }) => format!(
            "Added the skill “{name}” to this project, in {}, beside its other changed files. \
             It is offered from the person's next message.",
            project_skill(name)
        ),
        (Place::Everywhere, Change::AddSkill { .. }) => format!(
            "Added the skill “{name}” to the person's own skills, for all their projects. It \
             is offered from their next message."
        ),
        (Place::Project, Change::AddServer { .. }) => format!(
            "Added “{name}” to this project's .mcp.json. Its tools come from the person's next \
             message, once they trust the project's servers."
        ),
        (Place::Everywhere, Change::AddServer { .. }) => format!(
            "Added “{name}” to the person's own tool servers, for all their projects. Its tools \
             come from their next message; if it asks them to sign in, they connect it first."
        ),
        (Place::Project, Change::RemoveSkill { .. }) => {
            format!("Removed the skill “{name}” from this project.")
        }
        (Place::Everywhere, Change::RemoveSkill { .. }) => {
            format!("Removed the skill “{name}” from the person's own skills.")
        }
        (Place::Project, Change::RemoveServer { .. }) => {
            format!("Disconnected “{name}”: it is out of this project's .mcp.json.")
        }
        (Place::Everywhere, Change::RemoveServer { .. }) => format!(
            "Disconnected “{name}”: it is out of the person's own tool servers. Their sign-in \
             to it, if any, stays theirs until they remove it."
        ),
    };
    format!("{said}{replacing}")
}

/// Makes `change` in the project, through its computer.
async fn in_project(workspace: &Workspace, change: &Change) -> Result<(), String> {
    match change {
        Change::AddSkill { name, text, .. } => {
            let path = project_skill(name);
            workspace.write_for_glave(&path, text.as_bytes()).await
        }
        Change::RemoveSkill { name } => {
            let folder = format!("{PROJECT_SKILLS}/{name}");
            // The name holds only letters, digits and hyphens, so it needs no quoting.
            let removed = workspace.quietly(&format!("rm -rf -- {folder}")).await;
            removed.map_err(|error| format!("Could not remove {folder}: {}", error.message()))?;
            match workspace.stat(&folder).await?.kind {
                None => Ok(()),
                Some(_) => Err(format!("Could not remove {folder}.")),
            }
        }
        Change::AddServer { .. } | Change::RemoveServer { .. } => {
            let text = project_servers(workspace).await?;
            let changed = servers_changed(&text, change)?;
            let written = workspace.write_for_glave(PROJECT_SERVERS, changed.as_bytes());
            written.await
        }
    }
}

/// Makes `change` in the person's own folder, `folder`.
fn in_own(folder: &Path, change: &Change) -> Result<(), String> {
    let failed = |path: &Path, error: std::io::Error| {
        format!("Could not change {}: {error}", path.display())
    };
    match change {
        Change::AddSkill { name, text, .. } => {
            let file = own_skill(folder, name);
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent).map_err(|e| failed(parent, e))?;
            }
            glave_ports::file::replace(&file, text.as_bytes()).map_err(|e| failed(&file, e))
        }
        Change::RemoveSkill { name } => {
            let skill = folder.join(OWN_SKILLS).join(name);
            std::fs::remove_dir_all(&skill).map_err(|e| failed(&skill, e))
        }
        Change::AddServer { .. } | Change::RemoveServer { .. } => {
            let path = folder.join(OWN_SERVERS);
            // A link is followed, so the file it points to is the one changed.
            let path = std::fs::canonicalize(&path).unwrap_or(path);
            let changed = servers_changed(&own_servers(folder)?, change)?;
            glave_ports::file::replace(&path, changed.as_bytes()).map_err(|e| failed(&path, e))
        }
    }
}

/// `text`, a `.mcp.json`'s, with `change`'s server added or taken out.
fn servers_changed(text: &str, change: &Change) -> Result<String, String> {
    let mut edited = Edited::read(text).map_err(unreadable)?;
    match change {
        Change::AddServer { name, server } => {
            let raw = RawValue::from_string(server.to_string()).map_err(|e| e.to_string())?;
            edited.servers.set(name, raw);
        }
        Change::RemoveServer { name } => edited.servers.retain(|each| each != name),
        Change::AddSkill { .. } | Change::RemoveSkill { .. } => {}
    }
    edited.text().map_err(|error| error.to_string())
}

/// Why a `.mcp.json` could not be changed: it does not read.
fn unreadable(error: impl std::fmt::Display) -> String {
    format!("Its tool servers' settings don't read, so nothing changed: {error}")
}

fn project_skill(name: &str) -> String {
    format!("{PROJECT_SKILLS}/{name}/{SKILL_MD}")
}

fn own_skill(folder: &Path, name: &str) -> PathBuf {
    folder.join(OWN_SKILLS).join(name).join(SKILL_MD)
}

/// The project's `.mcp.json`: empty when it has none.
async fn project_servers(workspace: &Workspace) -> Result<String, String> {
    match workspace.read(PROJECT_SERVERS).await {
        Ok(bytes) => Ok(String::from_utf8_lossy(bytes.as_slice()).into_owned()),
        Err(ComputerError::NotFound(_)) => Ok(String::new()),
        Err(error) => Err(format!(
            "Could not read {PROJECT_SERVERS}: {}",
            error.message()
        )),
    }
}

/// The person's own `mcp.json` in `folder`: empty when they have none.
fn own_servers(folder: &Path) -> Result<String, String> {
    let path = folder.join(OWN_SERVERS);
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(format!("Could not read {}: {error}", path.display())),
    }
}

/// Runs `work`, which reads and writes files here, off the async runtime.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| error.to_string())?
}
