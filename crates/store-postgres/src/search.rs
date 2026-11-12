//! The search index's sealed rows read back and opened for the cache that searches them
//! (`glave_ports::search::SearchCache`, `specs/storage.md`, "Search index").

use glave_event::ThreadId;
use glave_ports::search::{Fingerprint, Found, IndexRow, IndexRows, SQUARES_MODULUS};
use glave_ports::{LogError, Scope};
use tokio_postgres::IsolationLevel;
use uuid::Uuid;

use crate::log::{PostgresLog, database, storage, stored_seq};
use crate::store::connection;

/// The newest rows after `$3` whose sealed bytes come to at most `$4`, oldest first.
const ROWS: &str = "
    SELECT pos, thread, seq, key_id, terms FROM (
        SELECT pos, thread, seq, key_id, terms,
               SUM(octet_length(terms)) OVER (ORDER BY pos DESC) AS upto
        FROM event_search WHERE org = $1 AND project = $2 AND pos > $3
    ) AS newest WHERE upto <= $4 ORDER BY pos";

/// How many rows, the sum of their positions, and of their squares modulo `$3`.
const FINGERPRINT: &str = "
    SELECT COUNT(*), COALESCE(SUM(pos), 0)::bigint,
           COALESCE(SUM(((pos % $3) * (pos % $3)) % $3), 0)::bigint
    FROM event_search WHERE org = $1 AND project = $2";

impl IndexRows for PostgresLog {
    /// Read in one repeatable-read snapshot, so the fingerprint and rows agree.
    async fn rows(
        &self,
        scope: &Scope,
        after: i64,
        most: u64,
    ) -> Result<(Fingerprint, Vec<IndexRow>), LogError> {
        let (org, project) = (scope.org().as_uuid(), scope.project().as_uuid());
        let most = i64::try_from(most).unwrap_or(i64::MAX);
        let mut client = connection(&self.pool, scope.org()).await.map_err(storage)?;
        let transaction = client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .await
            .map_err(database)?;
        let print = transaction
            .query_one(FINGERPRINT, &[&org, &project, &SQUARES_MODULUS])
            .await
            .map_err(database)?;
        let statement = transaction.prepare_cached(ROWS).await.map_err(database)?;
        let stored = transaction
            .query(&statement, &[&org, &project, &after, &most])
            .await
            .map_err(database)?;
        transaction.commit().await.map_err(database)?;
        drop(client);
        let count: i64 = print.try_get(0).map_err(LogError::corrupt)?;
        let print = Fingerprint {
            rows: u64::try_from(count).map_err(LogError::corrupt)?,
            sum: print.try_get(1).map_err(LogError::corrupt)?,
            squares: print.try_get(2).map_err(LogError::corrupt)?,
        };
        let mut rows = Vec::with_capacity(stored.len());
        for row in &stored {
            let thread: Uuid = row.try_get("thread").map_err(LogError::corrupt)?;
            let seq: i64 = row.try_get("seq").map_err(LogError::corrupt)?;
            let found = Found {
                thread: ThreadId::try_from(thread).map_err(LogError::corrupt)?,
                seq: stored_seq(u64::try_from(seq).map_err(LogError::corrupt)?)?,
            };
            let pos: i64 = row.try_get("pos").map_err(LogError::corrupt)?;
            let key_id: String = row.try_get("key_id").map_err(LogError::corrupt)?;
            let sealed: Vec<u8> = row.try_get("terms").map_err(LogError::corrupt)?;
            rows.push(
                self.keys
                    .open_row(scope, (pos, found), &key_id, &sealed)
                    .await?,
            );
        }
        Ok((print, rows))
    }
}
