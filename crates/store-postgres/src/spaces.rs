//! Spaces on Postgres: one row per space, its members in order beside it, and the folder keys
//! that lead to it.

use deadpool_postgres::{GenericClient, Pool};
use glave_event::{At, Principal};
use glave_ports::{BoxFuture, FolderKey, OrgId, Space, SpaceError, SpaceId, SpaceKind, Spaces};
use jiff::Timestamp;
use tokio_postgres::Row;
use uuid::Uuid;

use crate::store::{connection, database};

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
        .map_err(database(SpaceError::Storage))?
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
        .map_err(database(SpaceError::Storage))?;
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
        .map_err(database(SpaceError::Storage))?;
    for (position, member) in (0_i64..).zip(&space.members) {
        client
            .execute(
                "INSERT INTO space_members (org, space, position, member) VALUES ($1, $2, $3, $4)",
                &[&org, &id, &position, &principal_text(member)?],
            )
            .await
            .map_err(database(SpaceError::Storage))?;
    }
    Ok(())
}

/// Holds `owner`'s projects in `org` until the transaction `client` is in ends, so a name read
/// free in it is still free when it is written.
async fn hold_names<C: GenericClient>(
    client: &C,
    org: Uuid,
    owner: &str,
) -> Result<(), SpaceError> {
    client
        .execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
            &[&format!("glave/space-names/{org}/{owner}")],
        )
        .await
        .map_err(database(SpaceError::Storage))?;
    Ok(())
}

/// The ids and names of the projects `owner` (as stored) has in `org`.
async fn projects_of<C: GenericClient>(
    client: &C,
    org: Uuid,
    owner: &str,
) -> Result<Vec<(SpaceId, String)>, SpaceError> {
    client
        .query(
            "SELECT id, name FROM space_records
             WHERE org = $1 AND owner = $2 AND kind = 'project'",
            &[&org, &owner],
        )
        .await
        .map_err(database(SpaceError::Storage))?
        .iter()
        .map(|row| {
            let id: Uuid = row.try_get(0).map_err(corrupt)?;
            Ok((SpaceId::from_uuid(id), row.try_get(1).map_err(corrupt)?))
        })
        .collect()
}

/// Inserts `space`, a project under the next free name among its owner's, unless its id, or
/// its owner's personal space, is taken: the space as kept, if it was. Runs in a transaction.
async fn insert<C: GenericClient>(
    client: &C,
    org: Uuid,
    space: &Space,
) -> Result<Option<Space>, SpaceError> {
    Space::check_name(&space.name)?;
    let owner = principal_text(&space.owner)?;
    let mut space = space.clone();
    if space.kind == SpaceKind::Project {
        hold_names(client, org, &owner).await?;
        let taken: Vec<String> = projects_of(client, org, &owner)
            .await?
            .into_iter()
            .map(|(_, name)| name)
            .collect();
        space.name = Space::free_name(&space.name, &taken);
    }
    let space = &space;
    let inserted = client
        .execute(
            "INSERT INTO space_records (org, id, kind, name, owner, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT DO NOTHING",
            &[
                &org,
                &space.id.as_uuid(),
                &space.kind.as_str(),
                &space.name,
                &owner,
                &space.created_at.timestamp(),
                &space.updated_at.timestamp(),
            ],
        )
        .await
        .map_err(database(SpaceError::Storage))?;
    if inserted == 0 {
        return Ok(None);
    }
    write_members(client, org, space).await?;
    Ok(Some(space.clone()))
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
        let transaction = client
            .transaction()
            .await
            .map_err(database(SpaceError::Storage))?;
        insert(&transaction, org, new).await?;
        let row = transaction
            .query_one(
                &format!("{SELECT} AND owner = $2 AND kind = 'personal'"),
                &[&org, &principal_text(&new.owner)?],
            )
            .await
            .map_err(database(SpaceError::Storage))?;
        let space = assemble(&transaction, org, &row).await?;
        transaction
            .commit()
            .await
            .map_err(database(SpaceError::Storage))?;
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
        let transaction = client
            .transaction()
            .await
            .map_err(database(SpaceError::Storage))?;
        let mut found = None;
        for key in keys {
            let row = transaction
                .query_opt(
                    "SELECT space FROM space_folders WHERE org = $1 AND key = $2",
                    &[&org, &key.stored()],
                )
                .await
                .map_err(database(SpaceError::Storage))?;
            if let Some(row) = row {
                found = Some(row.try_get::<_, Uuid>(0).map_err(corrupt)?);
                break;
            }
        }
        let made = found.is_none();
        if made && insert(&transaction, org, new).await?.is_none() {
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
                .map_err(database(SpaceError::Storage))?;
        }
        // Another writer may have led the first key elsewhere meanwhile: theirs stands.
        if made && let Some(first) = keys.first() {
            let row = transaction
                .query_one(
                    "SELECT space FROM space_folders WHERE org = $1 AND key = $2",
                    &[&org, &first.stored()],
                )
                .await
                .map_err(database(SpaceError::Storage))?;
            let led: Uuid = row.try_get(0).map_err(corrupt)?;
            if led != id {
                transaction
                    .execute(
                        "DELETE FROM space_records WHERE org = $1 AND id = $2",
                        &[&org, &id],
                    )
                    .await
                    .map_err(database(SpaceError::Storage))?;
                id = led;
            }
        }
        let space = get_now(&transaction, org, id)
            .await?
            .ok_or_else(|| corrupt("a folder leads nowhere"))?;
        transaction
            .commit()
            .await
            .map_err(database(SpaceError::Storage))?;
        Ok(space)
    }
}

impl Spaces for PostgresSpaces {
    fn create<'a>(
        &'a self,
        org: OrgId,
        space: &'a Space,
    ) -> BoxFuture<'a, Result<Space, SpaceError>> {
        Box::pin(async move {
            let mut client = connection(&self.pool).await.map_err(unavailable)?;
            let transaction = client
                .transaction()
                .await
                .map_err(database(SpaceError::Storage))?;
            let Some(kept) = insert(&transaction, org.as_uuid(), space).await? else {
                return Err(SpaceError::Conflict(
                    "the space's id is taken, or its owner has a personal space already".into(),
                ));
            };
            transaction
                .commit()
                .await
                .map_err(database(SpaceError::Storage))?;
            Ok(kept)
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
                .map_err(database(SpaceError::Storage))?;
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
            let transaction = client
                .transaction()
                .await
                .map_err(database(SpaceError::Storage))?;
            let stored = transaction
                .query_opt(
                    "SELECT kind, owner FROM space_records WHERE org = $1 AND id = $2",
                    &[&org, &space.id.as_uuid()],
                )
                .await
                .map_err(database(SpaceError::Storage))?
                .ok_or(SpaceError::NotFound(space.id))?;
            let kind: String = stored.try_get(0).map_err(corrupt)?;
            let owner: String = stored.try_get(1).map_err(corrupt)?;
            if kind == SpaceKind::Project.as_str() {
                hold_names(&transaction, org, &owner).await?;
                let taken = projects_of(&transaction, org, &owner).await?;
                Space::check_free(space.id, &space.name, &taken)?;
            }
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
                .map_err(database(SpaceError::Storage))?;
            if changed == 0 {
                return Err(SpaceError::NotFound(space.id));
            }
            write_members(&transaction, org, space).await?;
            transaction
                .commit()
                .await
                .map_err(database(SpaceError::Storage))
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
