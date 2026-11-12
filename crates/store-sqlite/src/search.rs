//! The search index's sealed rows read back and opened for the cache that searches them
//! (`glave_ports::search::SearchCache`, `specs/storage.md`, "Search index").

use glave_event::{Seq, ThreadId};
use glave_ports::search::{Fingerprint, Found, IndexRow, IndexRows};
use glave_ports::{LogError, Scope};
use rusqlite::{Connection, params};

use crate::log::{SqliteLog, parse_uuid};
use crate::store::run;

/// A row as stored: its position, event, data key and sealed words.
type Stored = (i64, String, i64, String, Vec<u8>);

/// The newest rows after `?3` whose sealed bytes come to at most `?4`, oldest first.
const ROWS: &str = "
    SELECT pos, thread, seq, key_id, terms FROM (
        SELECT pos, thread, seq, key_id, terms,
               SUM(length(terms)) OVER (ORDER BY pos DESC) AS upto
        FROM event_search WHERE org = ?1 AND project = ?2 AND pos > ?3
    ) WHERE upto <= ?4 ORDER BY pos";

impl IndexRows for SqliteLog {
    /// Read in one transaction on the one connection, so the fingerprint and rows agree.
    async fn rows(
        &self,
        scope: &Scope,
        after: i64,
        most: u64,
    ) -> Result<(Fingerprint, Vec<IndexRow>), LogError> {
        let ids = [scope.org().to_string(), scope.project().to_string()];
        let most = i64::try_from(most).unwrap_or(i64::MAX);
        let (print, stored) = run(&self.connection, move |connection| {
            read(connection, &ids, after, most)
        })
        .await
        .map_err(|e| LogError::Storage(e.to_string()))?;
        let mut rows = Vec::with_capacity(stored.len());
        for (pos, thread, seq, key_id, sealed) in stored {
            let thread = ThreadId::try_from(parse_uuid(&thread)?).map_err(LogError::corrupt)?;
            let seq = Seq::new(u64::try_from(seq).map_err(LogError::corrupt)?)
                .map_err(LogError::corrupt)?;
            let found = Found { thread, seq };
            rows.push(
                self.keys
                    .open_row(scope, (pos, found), &key_id, &sealed)
                    .await?,
            );
        }
        Ok((print, rows))
    }
}

fn read(
    connection: &mut Connection,
    [org, project]: &[String; 2],
    after: i64,
    most: i64,
) -> rusqlite::Result<(Fingerprint, Vec<Stored>)> {
    let transaction = connection.transaction()?;
    let (rows, sum): (i64, i64) = transaction.query_row(
        "SELECT COUNT(*), COALESCE(SUM(pos), 0) FROM event_search WHERE org = ?1 AND project = ?2",
        params![org, project],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let stored = {
        let mut statement = transaction.prepare_cached(ROWS)?;
        let found = statement.query_map(params![org, project, after, most], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?;
        found.collect::<rusqlite::Result<Vec<Stored>>>()?
    };
    transaction.commit()?;
    let rows = u64::try_from(rows).unwrap_or_default();
    Ok((Fingerprint { rows, sum }, stored))
}
