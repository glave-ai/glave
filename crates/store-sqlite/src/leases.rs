//! Leases in the local database (`leases`), timed by the file's own clock: each a row with its
//! holder and when it lapses, taken, renewed and let go in one statement each, so two processes
//! on one file never both hold one.

use std::sync::Arc;
use std::time::Duration;

use glave_ports::{BoxFuture, Holder, Lease, LeaseError, Leased, Leases, OrgId, ttl_millis};
use jiff::Timestamp;
use rusqlite::{OptionalExtension as _, params};

use crate::store::{Shared, run};

/// The database's now, in milliseconds since 1970: the same within one statement.
const NOW: &str = "CAST(unixepoch('subsec') * 1000 AS INTEGER)";

/// The leases of the local database.
#[derive(Clone)]
pub struct SqliteLeases {
    connection: Shared,
}

impl SqliteLeases {
    pub(crate) fn new(connection: Shared) -> Self {
        Self { connection }
    }

    /// Runs `sql` with the lease's organization, key, holder and the milliseconds it lasts, and
    /// answers the lease with the expiry it returns, if it returned one.
    async fn kept(
        &self,
        sql: String,
        org: OrgId,
        on: Leased,
        holder: Holder,
        ttl: Duration,
    ) -> Result<Option<Lease>, LeaseError> {
        let millis = ttl_millis(ttl);
        let expires: Option<i64> = run(&self.connection, move |connection| {
            connection
                .prepare_cached(&sql)?
                .query_row(
                    params![org.to_string(), on.key(), holder.to_string(), millis],
                    |row| row.get(0),
                )
                .optional()
        })
        .await
        .map_err(storage)?;
        expires
            .map(|expires| {
                let expires = Timestamp::from_millisecond(expires).map_err(storage)?;
                Ok(Lease {
                    org,
                    on,
                    holder,
                    expires,
                })
            })
            .transpose()
    }
}

fn storage(error: impl std::fmt::Display) -> LeaseError {
    LeaseError::Storage(error.to_string())
}

impl Leases for SqliteLeases {
    fn acquire(
        &self,
        org: OrgId,
        on: Leased,
        holder: Holder,
        ttl: Duration,
    ) -> BoxFuture<'_, Result<Option<Lease>, LeaseError>> {
        let sql = format!(
            "INSERT INTO leases (org, key, holder, expires) VALUES (?1, ?2, ?3, {NOW} + ?4)
             ON CONFLICT (org, key) DO UPDATE SET
                 holder = excluded.holder, expires = excluded.expires
                 WHERE leases.holder = excluded.holder OR leases.expires <= {NOW}
             RETURNING expires"
        );
        Box::pin(self.kept(sql, org, on, holder, ttl))
    }

    fn renew<'a>(
        &'a self,
        lease: &'a Lease,
        ttl: Duration,
    ) -> BoxFuture<'a, Result<Option<Lease>, LeaseError>> {
        let sql = format!(
            "UPDATE leases SET expires = {NOW} + ?4
             WHERE org = ?1 AND key = ?2 AND holder = ?3
             RETURNING expires"
        );
        Box::pin(self.kept(sql, lease.org, lease.on, lease.holder, ttl))
    }

    fn release<'a>(&'a self, lease: &'a Lease) -> BoxFuture<'a, Result<(), LeaseError>> {
        let connection = Arc::clone(&self.connection);
        let (org, key, holder) = (lease.org, lease.on.key(), lease.holder);
        Box::pin(async move {
            run(&connection, move |connection| {
                connection
                    .prepare_cached(
                        "DELETE FROM leases WHERE org = ?1 AND key = ?2 AND holder = ?3",
                    )?
                    .execute(params![org.to_string(), key, holder.to_string()])
            })
            .await
            .map(drop)
            .map_err(storage)
        })
    }
}
