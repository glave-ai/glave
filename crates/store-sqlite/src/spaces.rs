//! Spaces on SQLite: one row per space, its members in order beside it, and the folder keys
//! that lead to it.

use glave_event::{At, Principal};
use glave_ports::{BoxFuture, FolderKey, OrgId, Space, SpaceError, SpaceId, SpaceKind, Spaces};
use rusqlite::{
    Connection, ErrorCode, OptionalExtension as _, Transaction, TransactionBehavior, params,
};
use uuid::Uuid;

use crate::store::{Shared, StoreError, run};

/// The spaces in the local database.
#[derive(Clone)]
pub struct SqliteSpaces {
    connection: Shared,
}

impl SqliteSpaces {
    pub(crate) fn new(connection: Shared) -> Self {
        Self { connection }
    }

    async fn with<T, F>(&self, work: F) -> Result<T, SpaceError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, Failure> + Send + 'static,
    {
        let outcome = run(&self.connection, move |connection| Ok(work(connection))).await;
        match outcome {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(Failure::Space(error))) => Err(error),
            Ok(Err(Failure::Database(error))) => Err(storage(&error)),
            Err(error) => Err(SpaceError::Storage(error.to_string())),
        }
    }
}

/// Why a call failed inside a transaction: the port's rule, or the database.
pub(crate) enum Failure {
    Space(SpaceError),
    Database(rusqlite::Error),
}

impl From<rusqlite::Error> for Failure {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

impl From<SpaceError> for Failure {
    fn from(error: SpaceError) -> Self {
        Self::Space(error)
    }
}

fn storage(error: &rusqlite::Error) -> SpaceError {
    SpaceError::Storage(error.to_string())
}

/// A principal as stored: its reference in JSON, written the same way every time.
pub(crate) fn principal_text(principal: &Principal) -> Result<String, SpaceError> {
    serde_json::to_string(principal).map_err(|e| SpaceError::Storage(e.to_string()))
}

fn corrupt(error: impl std::fmt::Display) -> Failure {
    Failure::Space(SpaceError::Storage(format!(
        "a stored space is corrupt: {error}"
    )))
}

/// A space's row, before its members are read.
struct Row {
    id: String,
    kind: String,
    name: String,
    owner: String,
    created_at: String,
    updated_at: String,
}

const COLUMNS: &str = "id, kind, name, owner, created_at, updated_at";

fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok(Row {
        id: row.get(0)?,
        kind: row.get(1)?,
        name: row.get(2)?,
        owner: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

/// The whole space `row` stands for, with its members.
fn assemble(connection: &Connection, org: &str, row: Row) -> Result<Space, Failure> {
    let mut statement = connection.prepare_cached(
        "SELECT member FROM space_members WHERE org = ?1 AND space = ?2 ORDER BY position",
    )?;
    let members = statement
        .query_map(params![org, row.id], |member| member.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    let members = members
        .iter()
        .map(|member| serde_json::from_str(member).map_err(corrupt))
        .collect::<Result<Vec<Principal>, Failure>>()?;
    Ok(Space {
        id: SpaceId::from_uuid(Uuid::parse_str(&row.id).map_err(corrupt)?),
        kind: SpaceKind::named(&row.kind)
            .ok_or_else(|| corrupt(format!("unknown kind {:?}", row.kind)))?,
        name: row.name,
        // A space made before records has no owner until the local person claims it: no one's.
        owner: if row.owner.is_empty() {
            Principal::glave()
        } else {
            serde_json::from_str(&row.owner).map_err(corrupt)?
        },
        members,
        created_at: At::try_from(row.created_at).map_err(corrupt)?,
        updated_at: At::try_from(row.updated_at).map_err(corrupt)?,
    })
}

fn get_now(connection: &Connection, org: &str, id: &str) -> Result<Option<Space>, Failure> {
    let found = connection
        .prepare_cached(&format!(
            "SELECT {COLUMNS} FROM space_records WHERE org = ?1 AND id = ?2"
        ))?
        .query_row(params![org, id], row)
        .optional()?;
    found
        .map(|found| assemble(connection, org, found))
        .transpose()
}

fn write_members(transaction: &Transaction<'_>, org: &str, space: &Space) -> Result<(), Failure> {
    let id = space.id.to_string();
    transaction.execute(
        "DELETE FROM space_members WHERE org = ?1 AND space = ?2",
        params![org, id],
    )?;
    for (position, member) in (0_i64..).zip(&space.members) {
        transaction.execute(
            "INSERT INTO space_members (org, space, position, member) VALUES (?1, ?2, ?3, ?4)",
            params![org, id, position, principal_text(member)?],
        )?;
    }
    Ok(())
}

/// The ids and names of the projects `owner` (as stored) has in `org`.
fn projects_of(
    transaction: &Transaction<'_>,
    org: &str,
    owner: &str,
) -> Result<Vec<(SpaceId, String)>, Failure> {
    let rows = transaction
        .prepare_cached(
            "SELECT id, name FROM space_records WHERE org = ?1 AND owner = ?2 AND kind = 'project'",
        )?
        .query_map(params![org, owner], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(id, name)| {
            Ok((
                SpaceId::from_uuid(Uuid::parse_str(&id).map_err(corrupt)?),
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

/// Keeps `space`, a project under the next free name among its owner's; answers it as kept.
fn insert(transaction: &Transaction<'_>, org: &str, space: &Space) -> Result<Space, Failure> {
    Space::check_name(&space.name)?;
    let owner = principal_text(&space.owner)?;
    let mut space = space.clone();
    if space.kind == SpaceKind::Project {
        let taken: Vec<String> = projects_of(transaction, org, &owner)?
            .into_iter()
            .map(|(_, name)| name)
            .collect();
        space.name = Space::free_name(&space.name, &taken);
    }
    let space = &space;
    let inserted = transaction.execute(
        "INSERT INTO space_records (org, id, kind, name, owner, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            org,
            space.id.to_string(),
            space.kind.as_str(),
            space.name,
            owner,
            space.created_at.to_string(),
            space.updated_at.to_string(),
        ],
    );
    match inserted {
        Ok(_) => {
            write_members(transaction, org, space)?;
            Ok(space.clone())
        }
        Err(rusqlite::Error::SqliteFailure(error, _))
            if error.code == ErrorCode::ConstraintViolation =>
        {
            Err(Failure::Space(SpaceError::Conflict(
                if space.kind == SpaceKind::Personal {
                    "this person has a personal space already, or the id is taken".to_owned()
                } else {
                    format!("space {} exists already", space.id)
                },
            )))
        }
        Err(error) => Err(error.into()),
    }
}

fn personal_now(connection: &mut Connection, org: &str, new: &Space) -> Result<Space, Failure> {
    if new.kind != SpaceKind::Personal {
        return Err(SpaceError::Invalid("a personal space must be personal".into()).into());
    }
    let owner = principal_text(&new.owner)?;
    let transaction = writing(connection)?;
    let found: Option<String> = transaction
        .query_row(
            "SELECT id FROM space_records WHERE org = ?1 AND owner = ?2 AND kind = 'personal'",
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
    let space =
        get_now(&transaction, org, &id)?.ok_or_else(|| corrupt("a personal space vanished"))?;
    transaction.commit()?;
    Ok(space)
}

fn folder_now(
    connection: &mut Connection,
    org: &str,
    keys: &[FolderKey],
    new: &Space,
) -> Result<Space, Failure> {
    let transaction = writing(connection)?;
    let mut found = None;
    for key in keys {
        found = transaction
            .query_row(
                "SELECT space FROM space_folders WHERE org = ?1 AND key = ?2",
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
            "INSERT OR IGNORE INTO space_folders (org, key, space) VALUES (?1, ?2, ?3)",
            params![org, key.stored(), id],
        )?;
    }
    let space =
        get_now(&transaction, org, &id)?.ok_or_else(|| corrupt("a folder leads nowhere"))?;
    transaction.commit()?;
    Ok(space)
}

impl Spaces for SqliteSpaces {
    fn create<'a>(
        &'a self,
        org: OrgId,
        space: &'a Space,
    ) -> BoxFuture<'a, Result<Space, SpaceError>> {
        let space = space.clone();
        Box::pin(self.with(move |connection| {
            let transaction = writing(connection)?;
            let kept = insert(&transaction, &org.to_string(), &space)?;
            transaction.commit()?;
            Ok(kept)
        }))
    }

    fn get(&self, org: OrgId, space: SpaceId) -> BoxFuture<'_, Result<Option<Space>, SpaceError>> {
        Box::pin(
            self.with(move |connection| get_now(connection, &org.to_string(), &space.to_string())),
        )
    }

    fn list<'a>(
        &'a self,
        org: OrgId,
        member: &'a Principal,
        limit: usize,
    ) -> BoxFuture<'a, Result<Vec<Space>, SpaceError>> {
        let member = principal_text(member);
        Box::pin(async move {
            let member = member?;
            let limit = i64::try_from(limit).unwrap_or(i64::MAX);
            self.with(move |connection| {
                let org = org.to_string();
                let rows = connection
                    .prepare_cached(&format!(
                        "SELECT {COLUMNS} FROM space_records r WHERE r.org = ?1
                           AND (r.owner = ?2 OR EXISTS (SELECT 1 FROM space_members m
                                WHERE m.org = r.org AND m.space = r.id AND m.member = ?2))
                         ORDER BY r.id LIMIT ?3"
                    ))?
                    .query_map(params![org, member, limit], row)?
                    .collect::<rusqlite::Result<Vec<Row>>>()?;
                rows.into_iter()
                    .map(|found| assemble(connection, &org, found))
                    .collect()
            })
            .await
        })
    }

    fn update<'a>(&'a self, org: OrgId, space: &'a Space) -> BoxFuture<'a, Result<(), SpaceError>> {
        let space = space.clone();
        Box::pin(self.with(move |connection| {
            Space::check_name(&space.name)?;
            let org = org.to_string();
            let transaction = writing(connection)?;
            let stored: Option<(String, String)> = transaction
                .query_row(
                    "SELECT kind, owner FROM space_records WHERE org = ?1 AND id = ?2",
                    params![org, space.id.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let Some((kind, owner)) = stored else {
                return Err(SpaceError::NotFound(space.id).into());
            };
            if kind == SpaceKind::Project.as_str() {
                let taken = projects_of(&transaction, &org, &owner)?;
                Space::check_free(space.id, &space.name, &taken)?;
            }
            let changed = transaction.execute(
                "UPDATE space_records SET name = ?3, updated_at = ?4 WHERE org = ?1 AND id = ?2",
                params![
                    org,
                    space.id.to_string(),
                    space.name,
                    space.updated_at.to_string()
                ],
            )?;
            if changed == 0 {
                return Err(SpaceError::NotFound(space.id).into());
            }
            write_members(&transaction, &org, &space)?;
            transaction.commit()?;
            Ok(())
        }))
    }

    fn personal<'a>(
        &'a self,
        org: OrgId,
        new: &'a Space,
    ) -> BoxFuture<'a, Result<Space, SpaceError>> {
        let new = new.clone();
        Box::pin(self.with(move |connection| personal_now(connection, &org.to_string(), &new)))
    }

    fn folder<'a>(
        &'a self,
        org: OrgId,
        keys: &'a [FolderKey],
        new: &'a Space,
    ) -> BoxFuture<'a, Result<Space, SpaceError>> {
        let (keys, new) = (keys.to_vec(), new.clone());
        Box::pin(self.with(move |connection| folder_now(connection, &org.to_string(), &keys, &new)))
    }
}

/// Gives every space made before records (no owner yet) to `owner`: on a person's own computer,
/// every such space was theirs.
pub(crate) async fn claim(connection: &Shared, owner: &Principal) -> Result<usize, StoreError> {
    let owner = principal_text(owner).map_err(|e| StoreError::Storage(e.to_string()))?;
    run(connection, move |connection| {
        connection.execute(
            "UPDATE space_records SET owner = ?1 WHERE owner = ''",
            params![owner],
        )
    })
    .await
}
