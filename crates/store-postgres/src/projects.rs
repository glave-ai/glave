//! Projects on Postgres: one row per project, its members in order beside it, and the folder keys
//! that lead to it.

use deadpool_postgres::{GenericClient, Pool};
use glave_event::{At, Principal};
use glave_ports::{
    BoxFuture, FolderKey, OrgId, Project, ProjectError, ProjectId, ProjectKind, Projects,
};
use jiff::Timestamp;
use tokio_postgres::Row;
use uuid::Uuid;

use crate::store::{connection, database};

/// The projects in the database.
#[derive(Clone)]
pub struct PostgresProjects {
    pool: Pool,
}

impl PostgresProjects {
    pub(crate) fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

fn corrupt(error: impl std::fmt::Display) -> ProjectError {
    ProjectError::Storage(format!("a stored project is corrupt: {error}"))
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "taken by value to pass straight to map_err"
)]
fn unavailable(error: crate::store::StoreError) -> ProjectError {
    ProjectError::Storage(error.to_string())
}

fn principal_text(principal: &Principal) -> Result<String, ProjectError> {
    serde_json::to_string(principal).map_err(|e| ProjectError::Storage(e.to_string()))
}

const SELECT: &str = "SELECT id, kind, name, owner, created_at, updated_at, revision
    FROM project_records WHERE org = $1";

/// The whole project `row` stands for, with its members.
async fn assemble<C: GenericClient>(
    client: &C,
    org: Uuid,
    row: &Row,
) -> Result<Project, ProjectError> {
    let id: Uuid = row.try_get(0).map_err(corrupt)?;
    let kind: String = row.try_get(1).map_err(corrupt)?;
    let owner: String = row.try_get(3).map_err(corrupt)?;
    let created: Timestamp = row.try_get(4).map_err(corrupt)?;
    let updated: Timestamp = row.try_get(5).map_err(corrupt)?;
    let revision: i64 = row.try_get(6).map_err(corrupt)?;
    let members = client
        .query(
            "SELECT member FROM project_members WHERE org = $1 AND project = $2 ORDER BY position",
            &[&org, &id],
        )
        .await
        .map_err(database(ProjectError::Storage))?
        .iter()
        .map(|member| {
            let text: String = member.try_get(0).map_err(corrupt)?;
            serde_json::from_str(&text).map_err(corrupt)
        })
        .collect::<Result<Vec<Principal>, ProjectError>>()?;
    Ok(Project {
        id: ProjectId::from_uuid(id),
        kind: ProjectKind::named(&kind).ok_or_else(|| corrupt(format!("unknown kind {kind:?}")))?,
        name: row.try_get(2).map_err(corrupt)?,
        owner: serde_json::from_str(&owner).map_err(corrupt)?,
        members,
        created_at: At::from(created),
        updated_at: At::from(updated),
        revision: u64::try_from(revision).map_err(corrupt)?,
    })
}

async fn get_now<C: GenericClient>(
    client: &C,
    org: Uuid,
    id: Uuid,
) -> Result<Option<Project>, ProjectError> {
    let row = client
        .query_opt(&format!("{SELECT} AND id = $2"), &[&org, &id])
        .await
        .map_err(database(ProjectError::Storage))?;
    match row {
        Some(row) => Ok(Some(assemble(client, org, &row).await?)),
        None => Ok(None),
    }
}

async fn write_members<C: GenericClient>(
    client: &C,
    org: Uuid,
    project: &Project,
) -> Result<(), ProjectError> {
    let id = project.id.as_uuid();
    client
        .execute(
            "DELETE FROM project_members WHERE org = $1 AND project = $2",
            &[&org, &id],
        )
        .await
        .map_err(database(ProjectError::Storage))?;
    for (position, member) in (0_i64..).zip(&project.members) {
        client
            .execute(
                "INSERT INTO project_members (org, project, position, member) VALUES ($1, $2, $3, $4)",
                &[&org, &id, &position, &principal_text(member)?],
            )
            .await
            .map_err(database(ProjectError::Storage))?;
    }
    Ok(())
}

/// Holds `owner`'s projects in `org` until the transaction `client` is in ends, so a name read
/// free in it is still free when it is written.
async fn hold_names<C: GenericClient>(
    client: &C,
    org: Uuid,
    owner: &str,
) -> Result<(), ProjectError> {
    client
        .execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
            &[&format!("glave/project-names/{org}/{owner}")],
        )
        .await
        .map_err(database(ProjectError::Storage))?;
    Ok(())
}

/// The ids and names of the projects `owner` (as stored) has in `org`.
async fn projects_of<C: GenericClient>(
    client: &C,
    org: Uuid,
    owner: &str,
) -> Result<Vec<(ProjectId, String)>, ProjectError> {
    client
        .query(
            "SELECT id, name FROM project_records
             WHERE org = $1 AND owner = $2 AND kind = 'project'",
            &[&org, &owner],
        )
        .await
        .map_err(database(ProjectError::Storage))?
        .iter()
        .map(|row| {
            let id: Uuid = row.try_get(0).map_err(corrupt)?;
            Ok((ProjectId::from_uuid(id), row.try_get(1).map_err(corrupt)?))
        })
        .collect()
}

/// Inserts `project`, a project under the next free name among its owner's, unless its id, or
/// its owner's personal project, is taken: the project as kept, if it was. Runs in a transaction.
async fn insert<C: GenericClient>(
    client: &C,
    org: Uuid,
    project: &Project,
) -> Result<Option<Project>, ProjectError> {
    Project::check_name(&project.name)?;
    let owner = principal_text(&project.owner)?;
    let mut project = project.clone();
    project.revision = 0;
    if project.kind == ProjectKind::Project {
        hold_names(client, org, &owner).await?;
        let taken: Vec<String> = projects_of(client, org, &owner)
            .await?
            .into_iter()
            .map(|(_, name)| name)
            .collect();
        project.name = Project::free_name(&project.name, &taken);
    }
    let project = &project;
    let inserted = client
        .execute(
            "INSERT INTO project_records (org, id, kind, name, owner, created_at, updated_at, revision)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 0) ON CONFLICT DO NOTHING",
            &[
                &org,
                &project.id.as_uuid(),
                &project.kind.as_str(),
                &project.name,
                &owner,
                &project.created_at.timestamp(),
                &project.updated_at.timestamp(),
            ],
        )
        .await
        .map_err(database(ProjectError::Storage))?;
    if inserted == 0 {
        return Ok(None);
    }
    write_members(client, org, project).await?;
    Ok(Some(project.clone()))
}

impl PostgresProjects {
    async fn personal_now(&self, org: OrgId, new: &Project) -> Result<Project, ProjectError> {
        if new.kind != ProjectKind::Personal {
            return Err(ProjectError::Invalid(
                "a personal project must be personal".into(),
            ));
        }
        let org = org.as_uuid();
        let mut client = connection(&self.pool).await.map_err(unavailable)?;
        let transaction = client
            .transaction()
            .await
            .map_err(database(ProjectError::Storage))?;
        insert(&transaction, org, new).await?;
        let row = transaction
            .query_one(
                &format!("{SELECT} AND owner = $2 AND kind = 'personal'"),
                &[&org, &principal_text(&new.owner)?],
            )
            .await
            .map_err(database(ProjectError::Storage))?;
        let project = assemble(&transaction, org, &row).await?;
        transaction
            .commit()
            .await
            .map_err(database(ProjectError::Storage))?;
        Ok(project)
    }

    async fn folder_now(
        &self,
        org: OrgId,
        keys: &[FolderKey],
        new: &Project,
    ) -> Result<Project, ProjectError> {
        let org = org.as_uuid();
        let mut client = connection(&self.pool).await.map_err(unavailable)?;
        let transaction = client
            .transaction()
            .await
            .map_err(database(ProjectError::Storage))?;
        let mut found = None;
        for key in keys {
            let row = transaction
                .query_opt(
                    "SELECT project FROM project_folders WHERE org = $1 AND key = $2",
                    &[&org, &key.stored()],
                )
                .await
                .map_err(database(ProjectError::Storage))?;
            if let Some(row) = row {
                found = Some(row.try_get::<_, Uuid>(0).map_err(corrupt)?);
                break;
            }
        }
        let made = found.is_none();
        if made && insert(&transaction, org, new).await?.is_none() {
            return Err(ProjectError::Conflict(format!(
                "project {} exists already",
                new.id
            )));
        }
        let mut id = found.unwrap_or_else(|| new.id.as_uuid());
        for key in keys {
            transaction
                .execute(
                    "INSERT INTO project_folders (org, key, project) VALUES ($1, $2, $3)
                     ON CONFLICT DO NOTHING",
                    &[&org, &key.stored(), &id],
                )
                .await
                .map_err(database(ProjectError::Storage))?;
        }
        // Another writer may have led the first key elsewhere meanwhile: theirs stands.
        if made && let Some(first) = keys.first() {
            let row = transaction
                .query_one(
                    "SELECT project FROM project_folders WHERE org = $1 AND key = $2",
                    &[&org, &first.stored()],
                )
                .await
                .map_err(database(ProjectError::Storage))?;
            let led: Uuid = row.try_get(0).map_err(corrupt)?;
            if led != id {
                transaction
                    .execute(
                        "DELETE FROM project_records WHERE org = $1 AND id = $2",
                        &[&org, &id],
                    )
                    .await
                    .map_err(database(ProjectError::Storage))?;
                id = led;
            }
        }
        let project = get_now(&transaction, org, id)
            .await?
            .ok_or_else(|| corrupt("a folder leads nowhere"))?;
        transaction
            .commit()
            .await
            .map_err(database(ProjectError::Storage))?;
        Ok(project)
    }
}

impl Projects for PostgresProjects {
    fn create<'a>(
        &'a self,
        org: OrgId,
        project: &'a Project,
    ) -> BoxFuture<'a, Result<Project, ProjectError>> {
        Box::pin(async move {
            let mut client = connection(&self.pool).await.map_err(unavailable)?;
            let transaction = client
                .transaction()
                .await
                .map_err(database(ProjectError::Storage))?;
            let Some(kept) = insert(&transaction, org.as_uuid(), project).await? else {
                return Err(ProjectError::Conflict(
                    "the project's id is taken, or its owner has a personal project already".into(),
                ));
            };
            transaction
                .commit()
                .await
                .map_err(database(ProjectError::Storage))?;
            Ok(kept)
        })
    }

    fn get(
        &self,
        org: OrgId,
        project: ProjectId,
    ) -> BoxFuture<'_, Result<Option<Project>, ProjectError>> {
        Box::pin(async move {
            let client = connection(&self.pool).await.map_err(unavailable)?;
            get_now(&client, org.as_uuid(), project.as_uuid()).await
        })
    }

    fn list<'a>(
        &'a self,
        org: OrgId,
        member: &'a Principal,
        limit: usize,
    ) -> BoxFuture<'a, Result<Vec<Project>, ProjectError>> {
        Box::pin(async move {
            let member = principal_text(member)?;
            let limit = i64::try_from(limit).unwrap_or(i64::MAX);
            let org = org.as_uuid();
            let client = connection(&self.pool).await.map_err(unavailable)?;
            let rows = client
                .query(
                    &format!(
                        "{SELECT} AND (owner = $2 OR EXISTS (SELECT 1 FROM project_members m
                            WHERE m.org = project_records.org AND m.project = project_records.id
                              AND m.member = $2))
                         ORDER BY id LIMIT $3"
                    ),
                    &[&org, &member, &limit],
                )
                .await
                .map_err(database(ProjectError::Storage))?;
            let mut projects = Vec::with_capacity(rows.len());
            for row in &rows {
                projects.push(assemble(&client, org, row).await?);
            }
            Ok(projects)
        })
    }

    fn update<'a>(
        &'a self,
        org: OrgId,
        project: &'a Project,
    ) -> BoxFuture<'a, Result<Project, ProjectError>> {
        Box::pin(async move {
            Project::check_name(&project.name)?;
            let revision =
                i64::try_from(project.revision).map_err(|_| ProjectError::Changed(project.id))?;
            let org = org.as_uuid();
            let mut client = connection(&self.pool).await.map_err(unavailable)?;
            let transaction = client
                .transaction()
                .await
                .map_err(database(ProjectError::Storage))?;
            let stored = transaction
                .query_opt(
                    "SELECT kind, owner, revision FROM project_records WHERE org = $1 AND id = $2",
                    &[&org, &project.id.as_uuid()],
                )
                .await
                .map_err(database(ProjectError::Storage))?
                .ok_or(ProjectError::NotFound(project.id))?;
            let kind: String = stored.try_get(0).map_err(corrupt)?;
            let owner: String = stored.try_get(1).map_err(corrupt)?;
            if stored.try_get::<_, i64>(2).map_err(corrupt)? != revision {
                return Err(ProjectError::Changed(project.id));
            }
            if kind == ProjectKind::Project.as_str() {
                hold_names(&transaction, org, &owner).await?;
                let taken = projects_of(&transaction, org, &owner).await?;
                Project::check_free(project.id, &project.name, &taken)?;
            }
            let changed = transaction
                .execute(
                    "UPDATE project_records SET name = $3, updated_at = $4, revision = revision + 1
                     WHERE org = $1 AND id = $2 AND revision = $5",
                    &[
                        &org,
                        &project.id.as_uuid(),
                        &project.name,
                        &project.updated_at.timestamp(),
                        &revision,
                    ],
                )
                .await
                .map_err(database(ProjectError::Storage))?;
            // Another update took the row first, between the read and this write.
            if changed == 0 {
                return Err(ProjectError::Changed(project.id));
            }
            write_members(&transaction, org, project).await?;
            let kept = get_now(&transaction, org, project.id.as_uuid())
                .await?
                .ok_or(ProjectError::NotFound(project.id))?;
            transaction
                .commit()
                .await
                .map_err(database(ProjectError::Storage))?;
            Ok(kept)
        })
    }

    fn personal<'a>(
        &'a self,
        org: OrgId,
        new: &'a Project,
    ) -> BoxFuture<'a, Result<Project, ProjectError>> {
        Box::pin(self.personal_now(org, new))
    }

    fn folder<'a>(
        &'a self,
        org: OrgId,
        keys: &'a [FolderKey],
        new: &'a Project,
    ) -> BoxFuture<'a, Result<Project, ProjectError>> {
        Box::pin(self.folder_now(org, keys, new))
    }
}
