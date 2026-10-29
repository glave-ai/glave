//! The sweep (`specs/storage.md`, "Blobs"): records no thread refers to deleted in one
//! transaction (those referred to once, and uploads a day old), then their bytes, then bytes
//! no record names once they are a day old. The candidates are locked before they are deleted,
//! and each delete checks the references again in a statement of its own, so a reference taken
//! at once is either seen or finds the record gone.

use std::collections::BTreeSet;

use glave_ports::{BlobError, OrgId, ProjectId, Scope, Swept, UNREFERENCED_FOR_MS};
use uuid::Uuid;

use super::{PostgresBlobs, storage};
use crate::store::{connection, database};

/// What a record no thread refers to may be deleted for: referred to once, or an upload older
/// than `$2`.
const UNREFERENCED: &str = "b.org = $1
    AND NOT EXISTS (SELECT 1 FROM blob_threads AS t
        WHERE t.org = b.org AND t.project = b.project AND t.name = b.name)
    AND (b.referred OR b.uploaded < $2)";

impl PostgresBlobs {
    pub(super) async fn sweep_now(&self, org: OrgId) -> Result<Swept, BlobError> {
        let before = self
            .clock
            .now()
            .as_millisecond()
            .saturating_sub(UNREFERENCED_FOR_MS);
        let (gone, kept) = self.delete_unreferenced(org, before).await?;
        let mut swept = Swept::default();
        for (project, object) in gone {
            let scope = Scope::glaves_own(org, ProjectId::from_uuid(project));
            self.forget(scope, &object).await;
            swept.blobs += 1;
        }
        for listed in self.objects.list(org).await? {
            let named = kept.contains(&(listed.scope.project().as_uuid(), listed.object.clone()));
            if !named && listed.written_ms < before {
                self.objects.delete(listed.scope, &listed.object).await?;
                swept.orphans += 1;
            }
        }
        Ok(swept)
    }

    /// Deletes the records no thread refers to, and answers their projects and objects, and
    /// those of every record left.
    async fn delete_unreferenced(
        &self,
        org: OrgId,
        before: i64,
    ) -> Result<(Vec<(Uuid, String)>, BTreeSet<(Uuid, String)>), BlobError> {
        let org = org.as_uuid();
        let mut client = connection(&self.pool, OrgId::from_uuid(org))
            .await
            .map_err(|e| storage(&e))?;
        let transaction = client
            .transaction()
            .await
            .map_err(database(BlobError::Storage))?;
        let candidates = transaction
            .query(
                &format!(
                    "SELECT b.project, b.name FROM blobs AS b WHERE {UNREFERENCED}
                     ORDER BY b.project, b.name FOR UPDATE"
                ),
                &[&org, &before],
            )
            .await
            .map_err(database(BlobError::Storage))?;
        let mut gone = Vec::new();
        for candidate in candidates {
            let project: Uuid = candidate.try_get(0).map_err(database(BlobError::Storage))?;
            let name: String = candidate.try_get(1).map_err(database(BlobError::Storage))?;
            let deleted = transaction
                .query_opt(
                    &format!(
                        "DELETE FROM blobs AS b WHERE {UNREFERENCED} AND b.project = $3
                         AND b.name = $4 RETURNING object"
                    ),
                    &[&org, &before, &project, &name],
                )
                .await
                .map_err(database(BlobError::Storage))?;
            if let Some(row) = deleted {
                gone.push((
                    project,
                    row.try_get(0).map_err(database(BlobError::Storage))?,
                ));
            }
        }
        let kept = transaction
            .query("SELECT project, object FROM blobs WHERE org = $1", &[&org])
            .await
            .map_err(database(BlobError::Storage))?
            .iter()
            .map(|row| Ok((row.try_get(0)?, row.try_get(1)?)))
            .collect::<Result<BTreeSet<_>, tokio_postgres::Error>>()
            .map_err(database(BlobError::Storage))?;
        transaction
            .commit()
            .await
            .map_err(database(BlobError::Storage))?;
        Ok((gone, kept))
    }
}
