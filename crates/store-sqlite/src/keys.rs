//! Wrapped keys in the local database: each organization's key, and each project's record of
//! keys, replaced whole by compare-and-swap on its revision.

use std::future::Future;

use glave_keys::{KeyError, KeyStorage, StoredProject};
use glave_ports::{OrgId, Scope};
use rusqlite::{OptionalExtension as _, params};

use crate::store::{Shared, run};

/// The key storage of the local database.
#[derive(Clone)]
pub struct SqliteKeys {
    connection: Shared,
}

impl SqliteKeys {
    pub(crate) fn new(connection: Shared) -> Self {
        Self { connection }
    }
}

fn storage(error: impl std::fmt::Display) -> KeyError {
    KeyError::Storage(error.to_string())
}

impl KeyStorage for SqliteKeys {
    fn org_key(
        &self,
        org: OrgId,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, KeyError>> + Send {
        let connection = self.connection.clone();
        async move {
            run(&connection, move |connection| {
                connection
                    .query_row(
                        "SELECT wrapped FROM org_keys WHERE org = ?1",
                        params![org.to_string()],
                        |row| row.get(0),
                    )
                    .optional()
            })
            .await
            .map_err(storage)
        }
    }

    fn add_org_key(
        &self,
        org: OrgId,
        wrapped: Vec<u8>,
    ) -> impl Future<Output = Result<Vec<u8>, KeyError>> + Send {
        let connection = self.connection.clone();
        async move {
            run(&connection, move |connection| {
                let transaction = connection.transaction()?;
                transaction.execute(
                    "INSERT OR IGNORE INTO org_keys (org, wrapped) VALUES (?1, ?2)",
                    params![org.to_string(), wrapped],
                )?;
                let kept = transaction.query_row(
                    "SELECT wrapped FROM org_keys WHERE org = ?1",
                    params![org.to_string()],
                    |row| row.get(0),
                )?;
                transaction.commit()?;
                Ok(kept)
            })
            .await
            .map_err(storage)
        }
    }

    fn project(
        &self,
        scope: &Scope,
    ) -> impl Future<Output = Result<Option<StoredProject>, KeyError>> + Send {
        let scope = *scope;
        let connection = self.connection.clone();
        async move {
            let row: Option<(i64, Vec<u8>)> = run(&connection, move |connection| {
                connection
                    .query_row(
                        "SELECT revision, record FROM project_keys WHERE org = ?1 AND project = ?2",
                        params![scope.org.to_string(), scope.project.to_string()],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()
            })
            .await
            .map_err(storage)?;
            row.map(|(revision, record)| {
                Ok(StoredProject {
                    revision: u64::try_from(revision).map_err(storage)?,
                    record,
                })
            })
            .transpose()
        }
    }

    fn save_project(
        &self,
        scope: &Scope,
        expected: Option<u64>,
        record: Vec<u8>,
    ) -> impl Future<Output = Result<bool, KeyError>> + Send {
        let scope = *scope;
        let connection = self.connection.clone();
        async move {
            let expected = expected
                .map(|revision| i64::try_from(revision).map_err(storage))
                .transpose()?;
            let (org, project) = (scope.org.to_string(), scope.project.to_string());
            let changed = run(&connection, move |connection| match expected {
                None => connection.execute(
                    "INSERT OR IGNORE INTO project_keys (org, project, revision, record)
                     VALUES (?1, ?2, 1, ?3)",
                    params![org, project, record],
                ),
                Some(revision) => connection.execute(
                    "UPDATE project_keys SET record = ?4, revision = revision + 1
                     WHERE org = ?1 AND project = ?2 AND revision = ?3",
                    params![org, project, revision, record],
                ),
            })
            .await
            .map_err(storage)?;
            Ok(changed == 1)
        }
    }
}
