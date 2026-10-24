//! Projects on SQLite: one row per project, and the folder keys that lead to it.

use glave_event::{At, Principal};
use glave_ports::{
    BoxFuture, FolderKey, OrgId, Project, ProjectError, ProjectId, ProjectKind, Projects,
};
use rusqlite::{
    Connection, ErrorCode, OptionalExtension as _, Transaction, TransactionBehavior, params,
};
use uuid::Uuid;

use crate::store::{Shared, run};

/// The projects in the local database.
#[derive(Clone)]
pub struct SqliteProjects {
    connection: Shared,
}

impl SqliteProjects {
    pub(crate) fn new(connection: Shared) -> Self {
        Self { connection }
    }

    async fn with<T, F>(&self, work: F) -> Result<T, ProjectError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, Failure> + Send + 'static,
    {
        let outcome = run(&self.connection, move |connection| Ok(work(connection))).await;
        match outcome {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(Failure::Project(error))) => Err(error),
            Ok(Err(Failure::Database(error))) => Err(storage(&error)),
            Err(error) => Err(ProjectError::Storage(error.to_string())),
        }
    }
}

/// Why a call failed inside a transaction: the port's rule, or the database.
pub(crate) enum Failure {
    Project(ProjectError),
    Database(rusqlite::Error),
}

impl From<rusqlite::Error> for Failure {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

impl From<ProjectError> for Failure {
    fn from(error: ProjectError) -> Self {
        Self::Project(error)
    }
}

fn storage(error: &rusqlite::Error) -> ProjectError {
    ProjectError::Storage(error.to_string())
}

/// A principal as stored: its reference in JSON, written the same way every time.
pub(crate) fn principal_text(principal: &Principal) -> Result<String, ProjectError> {
    serde_json::to_string(principal).map_err(|e| ProjectError::Storage(e.to_string()))
}

fn corrupt(error: impl std::fmt::Display) -> Failure {
    Failure::Project(ProjectError::Storage(format!(
        "a stored project is corrupt: {error}"
    )))
}

/// A project's row, as read.
struct Row {
    id: String,
    kind: String,
    name: String,
    owner: String,
    created_at: String,
    updated_at: String,
    revision: i64,
}

const COLUMNS: &str = "id, kind, name, owner, created_at, updated_at, revision";

fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok(Row {
        id: row.get(0)?,
        kind: row.get(1)?,
        name: row.get(2)?,
        owner: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        revision: row.get(6)?,
    })
}

/// The project `row` stands for.
fn assemble(row: Row) -> Result<Project, Failure> {
    Ok(Project {
        id: ProjectId::from_uuid(Uuid::parse_str(&row.id).map_err(corrupt)?),
        kind: ProjectKind::named(&row.kind)
            .ok_or_else(|| corrupt(format!("unknown kind {:?}", row.kind)))?,
        name: row.name,
        owner: serde_json::from_str(&row.owner).map_err(corrupt)?,
        created_at: At::try_from(row.created_at).map_err(corrupt)?,
        updated_at: At::try_from(row.updated_at).map_err(corrupt)?,
        revision: u64::try_from(row.revision).map_err(corrupt)?,
    })
}

fn get_now(connection: &Connection, org: &str, id: &str) -> Result<Option<Project>, Failure> {
    let found = connection
        .prepare_cached(&format!(
            "SELECT {COLUMNS} FROM project_records WHERE org = ?1 AND id = ?2"
        ))?
        .query_row(params![org, id], row)
        .optional()?;
    found.map(assemble).transpose()
}

/// The ids and names of the projects `owner` (as stored) has in `org`.
fn projects_of(
    transaction: &Transaction<'_>,
    org: &str,
    owner: &str,
) -> Result<Vec<(ProjectId, String)>, Failure> {
    let rows = transaction
        .prepare_cached(
            "SELECT id, name FROM project_records WHERE org = ?1 AND owner = ?2 AND kind = 'project'",
        )?
        .query_map(params![org, owner], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(id, name)| {
            Ok((
                ProjectId::from_uuid(Uuid::parse_str(&id).map_err(corrupt)?),
                name,
            ))
        })
        .collect()
}

/// A write transaction that holds the database from its start, so a name read in it is
/// still free when it is written.
fn writing(connection: &mut Connection) -> rusqlite::Result<Transaction<'_>> {
    connection.transaction_with_behavior(TransactionBehavior::Immediate)
}

/// Keeps `project`, a project under the next free name among its owner's; answers it as kept.
fn insert(transaction: &Transaction<'_>, org: &str, project: &Project) -> Result<Project, Failure> {
    Project::check_name(&project.name)?;
    let owner = principal_text(&project.owner)?;
    let mut project = project.clone();
    project.revision = 0;
    if project.kind == ProjectKind::Project {
        let taken: Vec<String> = projects_of(transaction, org, &owner)?
            .into_iter()
            .map(|(_, name)| name)
            .collect();
        project.name = Project::free_name(&project.name, &taken);
    }
    let project = &project;
    let inserted = transaction.execute(
        "INSERT INTO project_records (org, id, kind, name, owner, created_at, updated_at, revision)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
        params![
            org,
            project.id.to_string(),
            project.kind.as_str(),
            project.name,
            owner,
            project.created_at.to_string(),
            project.updated_at.to_string(),
        ],
    );
    match inserted {
        Ok(_) => Ok(project.clone()),
        Err(rusqlite::Error::SqliteFailure(error, _))
            if error.code == ErrorCode::ConstraintViolation =>
        {
            Err(Failure::Project(ProjectError::Conflict(
                if project.kind == ProjectKind::Personal {
                    "this person has a personal project already, or the id is taken".to_owned()
                } else {
                    format!("project {} exists already", project.id)
                },
            )))
        }
        Err(error) => Err(error.into()),
    }
}

fn personal_now(connection: &mut Connection, org: &str, new: &Project) -> Result<Project, Failure> {
    if new.kind != ProjectKind::Personal {
        return Err(ProjectError::Invalid("a personal project must be personal".into()).into());
    }
    let owner = principal_text(&new.owner)?;
    let transaction = writing(connection)?;
    let found: Option<String> = transaction
        .query_row(
            "SELECT id FROM project_records WHERE org = ?1 AND owner = ?2 AND kind = 'personal'",
            params![org, owner],
            |row| row.get(0),
        )
        .optional()?;
    let id = if let Some(id) = found {
        id
    } else {
        insert(&transaction, org, new)?;
        new.id.to_string()
    };
    let project =
        get_now(&transaction, org, &id)?.ok_or_else(|| corrupt("a personal project vanished"))?;
    transaction.commit()?;
    Ok(project)
}

fn folder_now(
    connection: &mut Connection,
    org: &str,
    keys: &[FolderKey],
    new: &Project,
) -> Result<Project, Failure> {
    let transaction = writing(connection)?;
    let mut found = None;
    for key in keys {
        found = transaction
            .query_row(
                "SELECT project FROM project_folders WHERE org = ?1 AND key = ?2",
                params![org, key.stored()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if found.is_some() {
            break;
        }
    }
    let id = if let Some(id) = found {
        id
    } else {
        insert(&transaction, org, new)?;
        new.id.to_string()
    };
    for key in keys {
        transaction.execute(
            "INSERT OR IGNORE INTO project_folders (org, key, project) VALUES (?1, ?2, ?3)",
            params![org, key.stored(), id],
        )?;
    }
    let project =
        get_now(&transaction, org, &id)?.ok_or_else(|| corrupt("a folder leads nowhere"))?;
    transaction.commit()?;
    Ok(project)
}

/// The project the first known of `keys` leads to in `org`, making nothing.
fn find_folder_now(
    connection: &Connection,
    org: &str,
    keys: &[FolderKey],
) -> Result<Option<Project>, Failure> {
    for key in keys {
        let found = connection
            .query_row(
                "SELECT project FROM project_folders WHERE org = ?1 AND key = ?2",
                params![org, key.stored()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if let Some(id) = found {
            return get_now(connection, org, &id);
        }
    }
    Ok(None)
}

impl Projects for SqliteProjects {
    fn create<'a>(
        &'a self,
        org: OrgId,
        project: &'a Project,
    ) -> BoxFuture<'a, Result<Project, ProjectError>> {
        let project = project.clone();
        Box::pin(self.with(move |connection| {
            let transaction = writing(connection)?;
            let kept = insert(&transaction, &org.to_string(), &project)?;
            transaction.commit()?;
            Ok(kept)
        }))
    }

    fn get(
        &self,
        org: OrgId,
        project: ProjectId,
    ) -> BoxFuture<'_, Result<Option<Project>, ProjectError>> {
        Box::pin(
            self.with(move |connection| {
                get_now(connection, &org.to_string(), &project.to_string())
            }),
        )
    }

    fn list<'a>(
        &'a self,
        org: OrgId,
        owner: &'a Principal,
        limit: usize,
    ) -> BoxFuture<'a, Result<Vec<Project>, ProjectError>> {
        let owner = principal_text(owner);
        Box::pin(async move {
            let owner = owner?;
            let limit = i64::try_from(limit).unwrap_or(i64::MAX);
            self.with(move |connection| {
                let rows = connection
                    .prepare_cached(&format!(
                        "SELECT {COLUMNS} FROM project_records WHERE org = ?1 AND owner = ?2
                         ORDER BY id LIMIT ?3"
                    ))?
                    .query_map(params![org.to_string(), owner, limit], row)?
                    .collect::<rusqlite::Result<Vec<Row>>>()?;
                rows.into_iter().map(assemble).collect()
            })
            .await
        })
    }

    fn update<'a>(
        &'a self,
        org: OrgId,
        project: &'a Project,
    ) -> BoxFuture<'a, Result<Project, ProjectError>> {
        let project = project.clone();
        Box::pin(self.with(move |connection| {
            Project::check_name(&project.name)?;
            let org = org.to_string();
            let transaction = writing(connection)?;
            let stored: Option<(String, String, i64)> = transaction
                .query_row(
                    "SELECT kind, owner, revision FROM project_records WHERE org = ?1 AND id = ?2",
                    params![org, project.id.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            let Some((kind, owner, revision)) = stored else {
                return Err(ProjectError::NotFound(project.id).into());
            };
            if u64::try_from(revision).ok() != Some(project.revision) {
                return Err(ProjectError::Changed(project.id).into());
            }
            if kind == ProjectKind::Project.as_str() {
                let taken = projects_of(&transaction, &org, &owner)?;
                Project::check_free(project.id, &project.name, &taken)?;
            }
            let changed = transaction.execute(
                "UPDATE project_records SET name = ?3, updated_at = ?4, revision = revision + 1
                 WHERE org = ?1 AND id = ?2",
                params![
                    org,
                    project.id.to_string(),
                    project.name,
                    project.updated_at.to_string()
                ],
            )?;
            if changed == 0 {
                return Err(ProjectError::NotFound(project.id).into());
            }
            let kept = get_now(&transaction, &org, &project.id.to_string())?
                .ok_or(ProjectError::NotFound(project.id))?;
            transaction.commit()?;
            Ok(kept)
        }))
    }

    fn personal<'a>(
        &'a self,
        org: OrgId,
        new: &'a Project,
    ) -> BoxFuture<'a, Result<Project, ProjectError>> {
        let new = new.clone();
        Box::pin(self.with(move |connection| personal_now(connection, &org.to_string(), &new)))
    }

    fn folder<'a>(
        &'a self,
        org: OrgId,
        keys: &'a [FolderKey],
        new: &'a Project,
    ) -> BoxFuture<'a, Result<Project, ProjectError>> {
        let (keys, new) = (keys.to_vec(), new.clone());
        Box::pin(self.with(move |connection| folder_now(connection, &org.to_string(), &keys, &new)))
    }

    fn find_folder<'a>(
        &'a self,
        org: OrgId,
        keys: &'a [FolderKey],
    ) -> BoxFuture<'a, Result<Option<Project>, ProjectError>> {
        let keys = keys.to_vec();
        Box::pin(self.with(move |connection| find_folder_now(connection, &org.to_string(), &keys)))
    }
}
