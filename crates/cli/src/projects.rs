//! The projects on this computer (`specs/storage.md`, "Projects"): the project a folder leads to,
//! found by its canonical path or, for a clone that moved or another checkout, by its `origin`
//! remote; and the person's personal project, where one-off chats and personal memory live.

use std::path::Path;

use glave_event::{At, Principal, PrincipalId};
use glave_ports::{
    Clock as _, FolderKey, OrgId, Project, ProjectId, ProjectKind, Scope, SystemClock,
};

use crate::data::Data;
use crate::system::new_uuid_v7;

/// The store's one organization.
async fn org(store: &Data) -> anyhow::Result<OrgId> {
    store.org().await
}

/// A new project record, owned by `owner`, made now.
fn new_project(kind: ProjectKind, name: String, owner: Principal) -> anyhow::Result<Project> {
    let now = At::from(SystemClock.now());
    Ok(Project {
        id: ProjectId::from_uuid(new_uuid_v7()?),
        kind,
        name,
        owner,
        members: Vec::new(),
        created_at: now,
        updated_at: now,
        revision: 0,
    })
}

/// The scope of the project `folder` leads to, for `person`: found by its path, else by
/// the repository's `origin` remote, else made as theirs.
pub async fn here(store: &Data, folder: &Path, person: PrincipalId) -> anyhow::Result<Scope> {
    let org = org(store).await?;
    let owner = Principal::person(person);
    let canonical = tokio::fs::canonicalize(folder)
        .await
        .unwrap_or_else(|_| folder.to_owned());
    let mut keys = vec![FolderKey::Path(canonical.to_string_lossy().into_owned())];
    if let Some(remote) = glave_config::trust::remote(&canonical).await {
        keys.push(FolderKey::Remote(remote));
    }
    let name = canonical.file_name().map_or_else(
        || canonical.to_string_lossy().into_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    let new = new_project(ProjectKind::Project, name, owner)?;
    let project = store.projects().folder(org, &keys, &new).await?;
    // Glave's own: the host opens the project of the folder it runs in, for its person.
    Ok(Scope::glaves_own(org, project.id))
}

/// The scope of `person`'s personal project, made the first time.
pub async fn personal(store: &Data, person: PrincipalId) -> anyhow::Result<Scope> {
    let org = org(store).await?;
    let new = new_project(
        ProjectKind::Personal,
        glave_session::PERSONAL_NAME.to_owned(),
        Principal::person(person),
    )?;
    let project = store.projects().personal(org, &new).await?;
    // Glave's own: the host keeps its person's own project.
    Ok(Scope::glaves_own(org, project.id))
}

/// The scope glave keeps a record of its own under `name`, such as the gateway's, which no
/// person works in.
pub async fn named(store: &Data, name: &str) -> anyhow::Result<Scope> {
    let org = org(store).await?;
    let new = new_project(ProjectKind::Project, name.to_owned(), Principal::glave())?;
    let keys = [FolderKey::Path(name.to_owned())];
    let project = store.projects().folder(org, &keys, &new).await?;
    // Glave's own: a record no person works in.
    Ok(Scope::glaves_own(org, project.id))
}
