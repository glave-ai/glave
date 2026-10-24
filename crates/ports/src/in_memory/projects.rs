//! Projects held in memory: the in-memory adapter of the projects port, for tests and for runs
//! that keep nothing.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

use glave_event::Principal;

use crate::projects::{FolderKey, Project, ProjectError, ProjectKind, Projects};
use crate::scope::{OrgId, ProjectId};
use crate::tools::BoxFuture;

#[derive(Debug, Default)]
struct Held {
    projects: BTreeMap<(OrgId, ProjectId), Project>,
    folders: BTreeMap<(OrgId, String), ProjectId>,
}

/// Projects held in memory. Nothing survives the process.
#[derive(Debug, Default)]
pub struct InMemoryProjects {
    held: Mutex<Held>,
}

impl InMemoryProjects {
    pub fn new() -> Self {
        Self::default()
    }

    fn held(&self) -> Result<MutexGuard<'_, Held>, ProjectError> {
        self.held
            .lock()
            .map_err(|_| ProjectError::Storage("the in-memory projects' lock is poisoned".into()))
    }

    fn create_now(&self, org: OrgId, project: &Project) -> Result<Project, ProjectError> {
        Project::check_name(&project.name)?;
        let mut held = self.held()?;
        insert(&mut held, org, project)
    }

    fn list_now(
        &self,
        org: OrgId,
        owner: &Principal,
        limit: usize,
    ) -> Result<Vec<Project>, ProjectError> {
        let held = self.held()?;
        Ok(held
            .projects
            .iter()
            .filter(|((at, _), project)| *at == org && project.owner == *owner)
            .map(|(_, project)| project.clone())
            .take(limit)
            .collect())
    }

    fn update_now(&self, org: OrgId, project: &Project) -> Result<Project, ProjectError> {
        Project::check_name(&project.name)?;
        let mut held = self.held()?;
        let stored = held
            .projects
            .get(&(org, project.id))
            .ok_or(ProjectError::NotFound(project.id))?;
        if stored.revision != project.revision {
            return Err(ProjectError::Changed(project.id));
        }
        if stored.kind == ProjectKind::Project {
            let taken = projects_of(&held, org, &stored.owner)
                .map(|other| (other.id, other.name.clone()))
                .collect::<Vec<_>>();
            Project::check_free(project.id, &project.name, &taken)?;
        }
        let kept = held
            .projects
            .get_mut(&(org, project.id))
            .ok_or(ProjectError::NotFound(project.id))?;
        kept.name.clone_from(&project.name);
        kept.updated_at = project.updated_at;
        kept.revision = kept.revision.saturating_add(1);
        Ok(kept.clone())
    }

    fn personal_now(&self, org: OrgId, new: &Project) -> Result<Project, ProjectError> {
        if new.kind != ProjectKind::Personal {
            return Err(ProjectError::Invalid(
                "a personal project must be personal".into(),
            ));
        }
        let mut held = self.held()?;
        if let Some(found) = personal_of(&held, org, &new.owner) {
            return Ok(found);
        }
        insert(&mut held, org, new)
    }

    fn folder_now(
        &self,
        org: OrgId,
        keys: &[FolderKey],
        new: &Project,
    ) -> Result<Project, ProjectError> {
        let mut held = self.held()?;
        let found = keys
            .iter()
            .find_map(|key| held.folders.get(&(org, key.stored())).copied());
        let project = if let Some(id) = found {
            held.projects
                .get(&(org, id))
                .cloned()
                .ok_or(ProjectError::NotFound(id))?
        } else {
            Project::check_name(&new.name)?;
            insert(&mut held, org, new)?
        };
        for key in keys {
            held.folders
                .entry((org, key.stored()))
                .or_insert(project.id);
        }
        Ok(project)
    }
}

impl InMemoryProjects {
    fn find_folder_now(
        &self,
        org: OrgId,
        keys: &[FolderKey],
    ) -> Result<Option<Project>, ProjectError> {
        let held = self.held()?;
        let found = keys
            .iter()
            .find_map(|key| held.folders.get(&(org, key.stored())).copied());
        Ok(found.and_then(|id| held.projects.get(&(org, id)).cloned()))
    }
}

fn personal_of(held: &Held, org: OrgId, owner: &Principal) -> Option<Project> {
    held.projects
        .iter()
        .find(|((at, _), project)| {
            *at == org && project.kind == ProjectKind::Personal && project.owner == *owner
        })
        .map(|(_, project)| project.clone())
}

/// The projects `owner` has in `org`.
fn projects_of<'a>(
    held: &'a Held,
    org: OrgId,
    owner: &'a Principal,
) -> impl Iterator<Item = &'a Project> {
    held.projects.iter().filter_map(move |((at, _), project)| {
        (*at == org && project.kind == ProjectKind::Project && project.owner == *owner)
            .then_some(project)
    })
}

/// Keeps `project`, a project under the next free name among its owner's; answers it as kept.
fn insert(held: &mut Held, org: OrgId, project: &Project) -> Result<Project, ProjectError> {
    if held.projects.contains_key(&(org, project.id)) {
        return Err(ProjectError::Conflict(format!(
            "project {} exists already",
            project.id
        )));
    }
    if project.kind == ProjectKind::Personal && personal_of(held, org, &project.owner).is_some() {
        return Err(ProjectError::Conflict(
            "this person has a personal project already".into(),
        ));
    }
    let mut kept = project.clone();
    kept.revision = 0;
    if kept.kind == ProjectKind::Project {
        let taken: Vec<String> = projects_of(held, org, &kept.owner)
            .map(|other| other.name.clone())
            .collect();
        kept.name = Project::free_name(&kept.name, &taken);
    }
    held.projects.insert((org, project.id), kept.clone());
    Ok(kept)
}

impl Projects for InMemoryProjects {
    fn create<'a>(
        &'a self,
        org: OrgId,
        project: &'a Project,
    ) -> BoxFuture<'a, Result<Project, ProjectError>> {
        Box::pin(std::future::ready(self.create_now(org, project)))
    }

    fn get(
        &self,
        org: OrgId,
        project: ProjectId,
    ) -> BoxFuture<'_, Result<Option<Project>, ProjectError>> {
        let found = self
            .held()
            .map(|held| held.projects.get(&(org, project)).cloned());
        Box::pin(std::future::ready(found))
    }

    fn list<'a>(
        &'a self,
        org: OrgId,
        owner: &'a Principal,
        limit: usize,
    ) -> BoxFuture<'a, Result<Vec<Project>, ProjectError>> {
        Box::pin(std::future::ready(self.list_now(org, owner, limit)))
    }

    fn update<'a>(
        &'a self,
        org: OrgId,
        project: &'a Project,
    ) -> BoxFuture<'a, Result<Project, ProjectError>> {
        Box::pin(std::future::ready(self.update_now(org, project)))
    }

    fn personal<'a>(
        &'a self,
        org: OrgId,
        new: &'a Project,
    ) -> BoxFuture<'a, Result<Project, ProjectError>> {
        Box::pin(std::future::ready(self.personal_now(org, new)))
    }

    fn folder<'a>(
        &'a self,
        org: OrgId,
        keys: &'a [FolderKey],
        new: &'a Project,
    ) -> BoxFuture<'a, Result<Project, ProjectError>> {
        Box::pin(std::future::ready(self.folder_now(org, keys, new)))
    }

    fn find_folder<'a>(
        &'a self,
        org: OrgId,
        keys: &'a [FolderKey],
    ) -> BoxFuture<'a, Result<Option<Project>, ProjectError>> {
        Box::pin(std::future::ready(self.find_folder_now(org, keys)))
    }
}

#[cfg(test)]
mod tests {
    use super::InMemoryProjects;
    use crate::conformance;

    #[test]
    fn in_memory_projects_pass_the_projects_conformance_suite() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a current-thread runtime always builds");
        assert_eq!(
            runtime.block_on(conformance::projects(&InMemoryProjects::new())),
            Ok(())
        );
    }
}
