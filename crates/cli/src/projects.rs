//! The projects on this computer (`specs/storage.md`, "Projects"): the project a folder leads to,
//! found by its canonical path or, for a clone that moved or another checkout, by its `origin`
//! remote, and none for the folders that are never a project; and the person's personal
//! project, where one-off chats and personal memory live.

use std::path::{Path, PathBuf};

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

/// Whether `folder` is never a project, and what is done there the person's Chats: the home
/// folder, a folder above it, or the temporary folder (`specs/one-server.md`, "A folder's
/// project"). Every command asks this one rule.
pub(crate) fn never_a_project(folder: &Path) -> bool {
    let canonical = |path: PathBuf| std::fs::canonicalize(&path).unwrap_or(path);
    let folder = canonical(folder.to_owned());
    let home = std::env::var_os("HOME").map(|home| canonical(PathBuf::from(home)));
    let temporary = canonical(std::env::temp_dir());
    home.is_some_and(|home| home.starts_with(&folder)) || folder == temporary
}

/// The scope of the project `folder` leads to, for `person`, making nothing: the person's
/// Chats where the folder is never a project ([`never_a_project`]); else the project found by
/// its path or the repository's `origin` remote; else none.
pub(crate) async fn found(
    store: &Data,
    folder: &Path,
    person: PrincipalId,
) -> anyhow::Result<Option<Scope>> {
    if never_a_project(folder) {
        return personal(store, person).await.map(Some);
    }
    let org = org(store).await?;
    let (_, keys) = keys(folder).await;
    let project = store.projects().find_folder(org, &keys).await?;
    // Glave's own: the host opens the project of the folder it runs in, for its person.
    Ok(project.map(|project| Scope::glaves_own(org, project.id)))
}

/// The scope of the project `folder` leads to, for `person`: found by its path, else by
/// the repository's `origin` remote, else made as theirs.
pub(crate) async fn here(
    store: &Data,
    folder: &Path,
    person: PrincipalId,
) -> anyhow::Result<Scope> {
    let org = org(store).await?;
    let owner = Principal::person(person);
    let (canonical, keys) = keys(folder).await;
    let name = canonical.file_name().map_or_else(
        || canonical.to_string_lossy().into_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    let new = new_project(ProjectKind::Project, name, owner)?;
    let project = store.projects().folder(org, &keys, &new).await?;
    // Glave's own: the host opens the project of the folder it runs in, for its person.
    Ok(Scope::glaves_own(org, project.id))
}

/// `folder`'s canonical path, and the keys a project is found by: that path, then the
/// repository's `origin` remote, if it has one.
async fn keys(folder: &Path) -> (PathBuf, Vec<FolderKey>) {
    let canonical = tokio::fs::canonicalize(folder)
        .await
        .unwrap_or_else(|_| folder.to_owned());
    let mut keys = vec![FolderKey::Path(canonical.to_string_lossy().into_owned())];
    if let Some(remote) = glave_config::trust::remote(&canonical).await {
        keys.push(FolderKey::Remote(remote));
    }
    (canonical, keys)
}

/// The scope of `person`'s personal project, made the first time.
pub(crate) async fn personal(store: &Data, person: PrincipalId) -> anyhow::Result<Scope> {
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
pub(crate) async fn named(store: &Data, name: &str) -> anyhow::Result<Scope> {
    let org = org(store).await?;
    let new = new_project(ProjectKind::Project, name.to_owned(), Principal::glave())?;
    let keys = [FolderKey::Path(name.to_owned())];
    let project = store.projects().folder(org, &keys, &new).await?;
    // Glave's own: a record no person works in.
    Ok(Scope::glaves_own(org, project.id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_root_and_the_temporary_folder_are_never_projects() {
        assert!(never_a_project(Path::new("/")));
        assert!(never_a_project(&std::env::temp_dir()));
        let inside = glave_ports::testing::TempDir::new("never-a-project").unwrap();
        assert!(
            !never_a_project(&inside),
            "a folder in the temporary folder may be one"
        );
    }
}
