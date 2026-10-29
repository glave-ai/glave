//! The directory in the database (`identities`, `memberships`): which person each identity's
//! key leads to, and the organizations each person joined, in the order they joined. Ids and
//! hashes only, read before anyone's organization is known, on connections set to none. A
//! link and a join are one statement each, so servers racing on one database agree.

use deadpool_postgres::Pool;
use glave_event::PrincipalId;
use glave_ports::{BoxFuture, Directory, DirectoryError, IdentityKey, OrgId};
use uuid::Uuid;

use crate::store::{database, unscoped};

/// The directory in the database.
#[derive(Clone)]
pub struct PostgresDirectory {
    pool: Pool,
}

impl PostgresDirectory {
    pub(crate) fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

fn storage(error: impl std::fmt::Display) -> DirectoryError {
    DirectoryError::Storage(error.to_string())
}

fn person_of(uuid: Uuid) -> Result<PrincipalId, DirectoryError> {
    PrincipalId::try_from(uuid).map_err(storage)
}

impl Directory for PostgresDirectory {
    fn link<'a>(
        &'a self,
        identity: &'a IdentityKey,
        person: PrincipalId,
    ) -> BoxFuture<'a, Result<PrincipalId, DirectoryError>> {
        Box::pin(async move {
            let client = unscoped(&self.pool).await.map_err(storage)?;
            let row = client
                .query_one(
                    "INSERT INTO identities (identity, person) VALUES ($1, $2)
                     ON CONFLICT (identity) DO UPDATE SET person = identities.person
                     RETURNING person",
                    &[&identity.as_str(), &person.as_uuid()],
                )
                .await
                .map_err(database(DirectoryError::Storage))?;
            person_of(row.try_get(0).map_err(database(DirectoryError::Storage))?)
        })
    }

    fn find<'a>(
        &'a self,
        identity: &'a IdentityKey,
    ) -> BoxFuture<'a, Result<Option<PrincipalId>, DirectoryError>> {
        Box::pin(async move {
            let client = unscoped(&self.pool).await.map_err(storage)?;
            let row = client
                .query_opt(
                    "SELECT person FROM identities WHERE identity = $1",
                    &[&identity.as_str()],
                )
                .await
                .map_err(database(DirectoryError::Storage))?;
            row.map(|row| person_of(row.try_get(0).map_err(database(DirectoryError::Storage))?))
                .transpose()
        })
    }

    fn join(&self, person: PrincipalId, org: OrgId) -> BoxFuture<'_, Result<(), DirectoryError>> {
        Box::pin(async move {
            let client = unscoped(&self.pool).await.map_err(storage)?;
            client
                .execute(
                    "INSERT INTO memberships (person, org) VALUES ($1, $2)
                     ON CONFLICT (person, org) DO NOTHING",
                    &[&person.as_uuid(), &org.as_uuid()],
                )
                .await
                .map(drop)
                .map_err(database(DirectoryError::Storage))
        })
    }

    fn organizations(
        &self,
        person: PrincipalId,
    ) -> BoxFuture<'_, Result<Vec<OrgId>, DirectoryError>> {
        Box::pin(async move {
            let client = unscoped(&self.pool).await.map_err(storage)?;
            let rows = client
                .query(
                    "SELECT org FROM memberships WHERE person = $1 ORDER BY joined",
                    &[&person.as_uuid()],
                )
                .await
                .map_err(database(DirectoryError::Storage))?;
            rows.iter()
                .map(|row| {
                    let org: Uuid = row.try_get(0).map_err(database(DirectoryError::Storage))?;
                    Ok(OrgId::from_uuid(org))
                })
                .collect()
        })
    }
}
