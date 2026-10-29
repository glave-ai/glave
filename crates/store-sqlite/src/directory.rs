//! The directory in the local database (`identities`, `memberships`): which person each
//! identity's key leads to, and the organizations each person joined, by the order of their
//! rows. Ids and hashes only; a link and a join are one statement each.

use std::str::FromStr;
use std::sync::Arc;

use glave_event::PrincipalId;
use glave_ports::{BoxFuture, Directory, DirectoryError, IdentityKey, OrgId};
use rusqlite::{OptionalExtension as _, params};
use uuid::Uuid;

use crate::store::{Shared, run};

/// The directory of the local database.
#[derive(Clone)]
pub struct SqliteDirectory {
    connection: Shared,
}

impl SqliteDirectory {
    pub(crate) fn new(connection: Shared) -> Self {
        Self { connection }
    }
}

fn storage(error: impl std::fmt::Display) -> DirectoryError {
    DirectoryError::Storage(error.to_string())
}

fn person_of(text: &str) -> Result<PrincipalId, DirectoryError> {
    let uuid = Uuid::from_str(text).map_err(storage)?;
    PrincipalId::try_from(uuid).map_err(storage)
}

impl Directory for SqliteDirectory {
    fn link<'a>(
        &'a self,
        identity: &'a IdentityKey,
        person: PrincipalId,
    ) -> BoxFuture<'a, Result<PrincipalId, DirectoryError>> {
        let connection = Arc::clone(&self.connection);
        let identity = identity.as_str().to_owned();
        Box::pin(async move {
            let kept: String = run(&connection, move |connection| {
                connection
                    .prepare_cached(
                        "INSERT INTO identities (identity, person) VALUES (?1, ?2)
                         ON CONFLICT (identity) DO UPDATE SET person = identities.person
                         RETURNING person",
                    )?
                    .query_row(params![identity, person.to_string()], |row| row.get(0))
            })
            .await
            .map_err(storage)?;
            person_of(&kept)
        })
    }

    fn find<'a>(
        &'a self,
        identity: &'a IdentityKey,
    ) -> BoxFuture<'a, Result<Option<PrincipalId>, DirectoryError>> {
        let connection = Arc::clone(&self.connection);
        let identity = identity.as_str().to_owned();
        Box::pin(async move {
            let kept: Option<String> = run(&connection, move |connection| {
                connection
                    .prepare_cached("SELECT person FROM identities WHERE identity = ?1")?
                    .query_row(params![identity], |row| row.get(0))
                    .optional()
            })
            .await
            .map_err(storage)?;
            kept.as_deref().map(person_of).transpose()
        })
    }

    fn join(&self, person: PrincipalId, org: OrgId) -> BoxFuture<'_, Result<(), DirectoryError>> {
        let connection = Arc::clone(&self.connection);
        Box::pin(async move {
            run(&connection, move |connection| {
                connection
                    .prepare_cached(
                        "INSERT INTO memberships (person, org) VALUES (?1, ?2)
                         ON CONFLICT (person, org) DO NOTHING",
                    )?
                    .execute(params![person.to_string(), org.to_string()])
            })
            .await
            .map(drop)
            .map_err(storage)
        })
    }

    fn leave(&self, person: PrincipalId, org: OrgId) -> BoxFuture<'_, Result<(), DirectoryError>> {
        let connection = Arc::clone(&self.connection);
        Box::pin(async move {
            run(&connection, move |connection| {
                connection
                    .prepare_cached("DELETE FROM memberships WHERE person = ?1 AND org = ?2")?
                    .execute(params![person.to_string(), org.to_string()])
            })
            .await
            .map(drop)
            .map_err(storage)
        })
    }

    fn organizations(
        &self,
        person: PrincipalId,
    ) -> BoxFuture<'_, Result<Vec<OrgId>, DirectoryError>> {
        let connection = Arc::clone(&self.connection);
        Box::pin(async move {
            let kept: Vec<String> = run(&connection, move |connection| {
                connection
                    .prepare_cached(
                        "SELECT org FROM memberships WHERE person = ?1 ORDER BY joined",
                    )?
                    .query_map(params![person.to_string()], |row| row.get(0))?
                    .collect()
            })
            .await
            .map_err(storage)?;
            kept.iter()
                .map(|org| Uuid::from_str(org).map(OrgId::from_uuid).map_err(storage))
                .collect()
        })
    }
}
