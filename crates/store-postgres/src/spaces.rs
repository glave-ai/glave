//! Spaces on Postgres: one row per space, its members in order beside it, and the folder keys
//! that lead to it.

use deadpool_postgres::{GenericClient, Pool};
use glave_event::{At, Principal};
use glave_ports::{BoxFuture, FolderKey, OrgId, Space, SpaceError, SpaceId, SpaceKind, Spaces};
use jiff::Timestamp;
use tokio_postgres::Row;
use uuid::Uuid;

use crate::store::{connection, describe};

/// The spaces in the database.
#[derive(Clone)]
pub struct PostgresSpaces {
    pool: Pool,
}

impl PostgresSpaces {
    pub(crate) fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "taken by value to pass straight to map_err"
)]
fn database(error: tokio_postgres::Error) -> SpaceError {
    SpaceError::Storage(format!("the database failed: {}", describe(&error)))
}

fn corrupt(error: impl std::fmt::Display) -> SpaceError {
    SpaceError::Storage(format!("a stored space is corrupt: {error}"))
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "taken by value to pass straight to map_err"
)]
fn unavailable(error: crate::store::StoreError) -> SpaceError {
    SpaceError::Storage(error.to_string())
}

fn principal_text(principal: &Principal) -> Result<String, SpaceError> {
    serde_json::to_string(principal).map_err(|e| SpaceError::Storage(e.to_string()))
}

const SELECT: &str =
    "SELECT id, kind, name, owner, created_at, updated_at FROM space_records WHERE org = $1";

/// The whole space `row` stands for, with its members.
async fn assemble<C: GenericClient>(client: &C, org: Uuid, row: &Row) -> Result<Space, SpaceError> {
    let id: Uuid = row.try_get(0).map_err(corrupt)?;
    let kind: String = row.try_get(1).map_err(corrupt)?;
    let owner: String = row.try_get(3).map_err(corrupt)?;
    let created: Timestamp = row.try_get(4).map_err(corrupt)?;
    let updated: Timestamp = row.try_get(5).map_err(corrupt)?;
    let members = client
        .query(
            "SELECT member FROM space_members WHERE org = $1 AND space = $2 ORDER BY position",
            &[&org, &id],
        )
        .await
        .map_err(database)?
        .iter()
        .map(|member| {
            let text: String = member.try_get(0).map_err(corrupt)?;
            serde_json::from_str(&text).map_err(corrupt)
        })
        .collect::<Result<Vec<Principal>, SpaceError>>()?;
    Ok(Space {
        id: SpaceId::from_uuid(id),
        kind: SpaceKind::named(&kind).ok_or_else(|| corrupt(format!("unknown kind {kind:?}")))?,
        name: row.try_get(2).map_err(corrupt)?,
        owner: serde_json::from_str(&owner).map_err(corrupt)?,
        members,
        created_at: At::from(created),
        updated_at: At::from(updated),
    })
}

async fn get_now<C: GenericClient>(
    client: &C,
    org: Uuid,
    id: Uuid,
) -> Result<Option<Space>, SpaceError> {
    let row = client
        .query_opt(&format!("{SELECT} AND id = $2"), &[&org, &id])
        .await
        .map_err(database)?;
    match row {
        Some(row) => Ok(Some(assemble(client, org, &row).await?)),
        None => Ok(None),
    }
}

async fn write_members<C: GenericClient>(
    client: &C,
    org: Uuid,
    space: &Space,
) -> Result<(), SpaceError> {
    let id = space.id.as_uuid();
    client
        .execute(
            "DELETE FROM space_members WHERE org = $1 AND space = $2",
            &[&org, &id],
        )
        .await
        .map_err(database)?;
    for (position, member) in (0_i64..).zip(&space.members) {
        client
            .execute(
                "INSERT INTO space_members (org, space, position, member) VALUES ($1, $2, $3, $4)",
                &[&org, &id, &position, &principal_text(member)?],
            )
            .await
            .map_err(database)?;
    }
    Ok(())
}

/// Inserts `space` unless its id, or its owner's personal space, is taken: whether it did.
async fn insert<C: GenericClient>(
    client: &C,
    org: Uuid,
    space: &Space,
) -> Result<bool, SpaceError> {
    Space::check_name(&space.name)?;
    let inserted = client
        .execute(
            "INSERT INTO space_records (org, id, kind, name, owner, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT DO NOTHING",
            &[
                &org,
                &space.id.as_uuid(),
                &space.kind.as_str(),
                &space.name,
                &principal_text(&space.owner)?,
                &space.created_at.timestamp(),
                &space.updated_at.timestamp(),
            ],
        )
        .await
        .map_err(database)?;
    if inserted == 0 {
        return Ok(false);
    }
    write_members(client, org, space).await?;
    Ok(true)
}

impl PostgresSpaces {
    async fn personal_now(&self, org: OrgId, new: &Space) -> Result<Space, SpaceError> {
        if new.kind != SpaceKind::Personal {
            return Err(SpaceError::Invalid(
                "a personal space must be personal".into(),
            ));
        }
        let org = org.as_uuid();
        let mut client = connection(&self.pool).await.map_err(unavailable)?;
        let transaction = client.transaction().await.map_err(database)?;
        insert(&transaction, org, new).await?;
        let row = transaction
            .query_one(
                &format!("{SELECT} AND owner = $2 AND kind = 'personal'"),
                &[&org, &principal_text(&new.owner)?],
            )
            .await
            .map_err(database)?;
        let space = assemble(&transaction, org, &row).await?;
        transaction.commit().await.map_err(database)?;
        Ok(space)
    }

    async fn folder_now(
        &self,
        org: OrgId,
        keys: &[FolderKey],
        new: &Space,
    ) -> Result<Space, SpaceError> {
        let org = org.as_uuid();
        let mut client = connection(&self.pool).await.map_err(unavailable)?;
        let transaction = client.transaction().await.map_err(database)?;
        let mut found = None;
        for key in keys {
            let row = transaction
                .query_opt(
                    "SELECT space FROM space_folders WHERE org = $1 AND key = $2",
                    &[&org, &key.stored()],
                )
                .await
                .map_err(database)?;
            if let Some(row) = row {
                found = Some(row.try_get::<_, Uuid>(0).map_err(corrupt)?);
                break;
            }
        }
        let made = found.is_none();
        if made && !insert(&transaction, org, new).await? {
            return Err(SpaceError::Conflict(format!(
                "space {} exists already",
                new.id
            )));
        }
        let mut id = found.unwrap_or_else(|| new.id.as_uuid());
        for key in keys {
            transaction
                .execute(
                    "INSERT INTO space_folders (org, key, space) VALUES ($1, $2, $3)
                     ON CONFLICT DO NOTHING",
                    &[&org, &key.stored(), &id],
                )
                .await
                .map_err(database)?;
        }
        // Another writer may have led the first key elsewhere meanwhile: theirs stands.
        if made && let Some(first) = keys.first() {
            let row = transaction
                .query_one(
                    "SELECT space FROM space_folders WHERE org = $1 AND key = $2",
                    &[&org, &first.stored()],
                )
                .await
                .map_err(database)?;
            let led: Uuid = row.try_get(0).map_err(corrupt)?;
            if led != id {
                transaction
                    .execute(
                        "DELETE FROM space_records WHERE org = $1 AND id = $2",
                        &[&org, &id],
                    )
                    .await
                    .map_err(database)?;
                id = led;
            }
        }
        let space = get_now(&transaction, org, id)
            .await?
            .ok_or_else(|| corrupt("a folder leads nowhere"))?;
        transaction.commit().await.map_err(database)?;
        Ok(space)
    }
}

impl Spaces for PostgresSpaces {
    fn create<'a>(&'a self, org: OrgId, space: &'a Space) -> BoxFuture<'a, Result<(), SpaceError>> {
        Box::pin(async move {
            let client = connection(&self.pool).await.map_err(unavailable)?;
            if insert(&client, org.as_uuid(), space).await? {
                Ok(())
            } else {
                Err(SpaceError::Conflict(
                    "the space's id is taken, or its owner has a personal space already".into(),
                ))
            }
        })
    }

    fn get(&self, org: OrgId, space: SpaceId) -> BoxFuture<'_, Result<Option<Space>, SpaceError>> {
        Box::pin(async move {
            let client = connection(&self.pool).await.map_err(unavailable)?;
            get_now(&client, org.as_uuid(), space.as_uuid()).await
        })
    }

    fn list<'a>(
        &'a self,
        org: OrgId,
        member: &'a Principal,
        limit: usize,
    ) -> BoxFuture<'a, Result<Vec<Space>, SpaceError>> {
        Box::pin(async move {
            let member = principal_text(member)?;
            let limit = i64::try_from(limit).unwrap_or(i64::MAX);
            let org = org.as_uuid();
            let client = connection(&self.pool).await.map_err(unavailable)?;
            let rows = client
                .query(
                    &format!(
                        "{SELECT} AND (owner = $2 OR EXISTS (SELECT 1 FROM space_members m
                            WHERE m.org = space_records.org AND m.space = space_records.id
                              AND m.member = $2))
                         ORDER BY id LIMIT $3"
                    ),
                    &[&org, &member, &limit],
                )
                .await
                .map_err(database)?;
            let mut spaces = Vec::with_capacity(rows.len());
            for row in &rows {
                spaces.push(assemble(&client, org, row).await?);
            }
            Ok(spaces)
        })
    }

    fn update<'a>(&'a self, org: OrgId, space: &'a Space) -> BoxFuture<'a, Result<(), SpaceError>> {
        Box::pin(async move {
            Space::check_name(&space.name)?;
            let org = org.as_uuid();
            let mut client = connection(&self.pool).await.map_err(unavailable)?;
            let transaction = client.transaction().await.map_err(database)?;
            let changed = transaction
                .execute(
                    "UPDATE space_records SET name = $3, updated_at = $4
                     WHERE org = $1 AND id = $2",
                    &[
                        &org,
                        &space.id.as_uuid(),
                        &space.name,
                        &space.updated_at.timestamp(),
                    ],
                )
                .await
                .map_err(database)?;
            if changed == 0 {
                return Err(SpaceError::NotFound(space.id));
            }
            write_members(&transaction, org, space).await?;
            transaction.commit().await.map_err(database)
        })
    }

    fn personal<'a>(
        &'a self,
        org: OrgId,
        new: &'a Space,
    ) -> BoxFuture<'a, Result<Space, SpaceError>> {
        Box::pin(self.personal_now(org, new))
    }

    fn folder<'a>(
        &'a self,
        org: OrgId,
        keys: &'a [FolderKey],
        new: &'a Space,
    ) -> BoxFuture<'a, Result<Space, SpaceError>> {
        Box::pin(self.folder_now(org, keys, new))
    }
}
