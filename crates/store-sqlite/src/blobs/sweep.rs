//! The sweep (`specs/storage.md`, "Blobs"): records no thread refers to deleted in one
//! transaction (those referred to once, and uploads a day old), then their bytes, then bytes
//! no record names once they are a day old.

use std::collections::BTreeSet;

use glave_ports::{BlobError, OrgId, ProjectId, Scope, Swept, UNREFERENCED_FOR_MS};
use rusqlite::params;
use uuid::Uuid;

use super::{SqliteBlobs, storage};
use crate::store::run;

impl SqliteBlobs {
    pub(super) async fn sweep_now(&self, org: OrgId) -> Result<Swept, BlobError> {
        let before = self
            .clock
            .now()
            .as_millisecond()
            .saturating_sub(UNREFERENCED_FOR_MS);
        let (gone, kept) = run(&self.connection, move |connection| {
            let org = org.to_string();
            let transaction = connection.transaction()?;
            let gone: Vec<(String, String)> = transaction
                .prepare(
                    "DELETE FROM blobs AS b WHERE b.org = ?1
                     AND NOT EXISTS (SELECT 1 FROM blob_threads AS t
                         WHERE t.org = b.org AND t.project = b.project AND t.name = b.name)
                     AND (b.referred = 1 OR b.uploaded < ?2)
                     RETURNING project, object",
                )?
                .query_map(params![org, before], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            let kept: BTreeSet<(String, String)> = transaction
                .prepare("SELECT project, object FROM blobs WHERE org = ?1")?
                .query_map(params![org], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            transaction.commit()?;
            Ok((gone, kept))
        })
        .await
        .map_err(|e| storage(&e))?;
        let mut swept = Swept::default();
        for (project, object) in gone {
            if let Some(scope) = scope_of(org, &project) {
                self.forget(scope, &object).await;
                swept.blobs += 1;
            }
        }
        for listed in self.objects.list(org).await? {
            let named = kept.contains(&(listed.scope.project().to_string(), listed.object.clone()));
            if !named && listed.written_ms < before {
                self.objects.delete(listed.scope, &listed.object).await?;
                swept.orphans += 1;
            }
        }
        Ok(swept)
    }
}

/// The scope a stored project id names in `org`, if it is one.
fn scope_of(org: OrgId, project: &str) -> Option<Scope> {
    let project = Uuid::parse_str(project).ok()?;
    Some(Scope::glaves_own(org, ProjectId::from_uuid(project)))
}
