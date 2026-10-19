//! Wrapped keys in the database: each organization's key, and each project's record of keys,
//! replaced whole by compare-and-swap on its revision.

use std::future::Future;

use deadpool_postgres::Pool;
use glave_keys::{KeyError, KeyStorage, StoredProject};
use glave_ports::{OrgId, Scope};

use crate::store::{connection, database};

/// The key storage of the database.
#[derive(Clone)]
pub struct PostgresKeys {
    pool: Pool,
}

impl PostgresKeys {
    pub(crate) fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

fn storage(error: impl std::fmt::Display) -> KeyError {
    KeyError::Storage(error.to_string())
}

impl KeyStorage for PostgresKeys {
    fn org_key(
        &self,
        org: OrgId,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, KeyError>> + Send {
        let pool = self.pool.clone();
        async move {
            let client = connection(&pool).await.map_err(storage)?;
            let row = client
                .query_opt(
                    "SELECT wrapped FROM org_keys WHERE org = $1",
                    &[&org.as_uuid()],
                )
                .await
                .map_err(database(KeyError::Storage))?;
            row.map(|row| row.try_get(0))
                .transpose()
                .map_err(database(KeyError::Storage))
        }
    }

    fn add_org_key(
        &self,
        org: OrgId,
        wrapped: Vec<u8>,
    ) -> impl Future<Output = Result<Vec<u8>, KeyError>> + Send {
        let pool = self.pool.clone();
        async move {
            let client = connection(&pool).await.map_err(storage)?;
            client
                .execute(
                    "INSERT INTO org_keys (org, wrapped) VALUES ($1, $2) ON CONFLICT DO NOTHING",
                    &[&org.as_uuid(), &wrapped],
                )
                .await
                .map_err(database(KeyError::Storage))?;
            client
                .query_one(
                    "SELECT wrapped FROM org_keys WHERE org = $1",
                    &[&org.as_uuid()],
                )
                .await
                .map_err(database(KeyError::Storage))?
                .try_get(0)
                .map_err(database(KeyError::Storage))
        }
    }

    fn project(
        &self,
        scope: &Scope,
    ) -> impl Future<Output = Result<Option<StoredProject>, KeyError>> + Send {
        let scope = *scope;
        let pool = self.pool.clone();
        async move {
            let client = connection(&pool).await.map_err(storage)?;
            let row = client
                .query_opt(
                    "SELECT revision, record FROM project_keys WHERE org = $1 AND project = $2",
                    &[&scope.org().as_uuid(), &scope.project().as_uuid()],
                )
                .await
                .map_err(database(KeyError::Storage))?;
            let Some(row) = row else {
                return Ok(None);
            };
            let revision: i64 = row.try_get(0).map_err(database(KeyError::Storage))?;
            Ok(Some(StoredProject {
                revision: u64::try_from(revision).map_err(storage)?,
                record: row.try_get(1).map_err(database(KeyError::Storage))?,
            }))
        }
    }

    fn save_project(
        &self,
        scope: &Scope,
        expected: Option<u64>,
        record: Vec<u8>,
    ) -> impl Future<Output = Result<bool, KeyError>> + Send {
        let scope = *scope;
        let pool = self.pool.clone();
        async move {
            let (org, project) = (scope.org().as_uuid(), scope.project().as_uuid());
            let client = connection(&pool).await.map_err(storage)?;
            let changed = match expected {
                None => client
                    .execute(
                        "INSERT INTO project_keys (org, project, revision, record)
                         VALUES ($1, $2, 1, $3) ON CONFLICT DO NOTHING",
                        &[&org, &project, &record],
                    )
                    .await
                    .map_err(database(KeyError::Storage))?,
                Some(revision) => {
                    let revision = i64::try_from(revision).map_err(storage)?;
                    client
                        .execute(
                            "UPDATE project_keys SET record = $4, revision = revision + 1
                             WHERE org = $1 AND project = $2 AND revision = $3",
                            &[&org, &project, &revision, &record],
                        )
                        .await
                        .map_err(database(KeyError::Storage))?
                }
            };
            Ok(changed == 1)
        }
    }
}
