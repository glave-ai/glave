//! Leases in the database (`leases`), timed by the database's own clock: each a row with its
//! holder and when it lapses, taken, renewed and let go in one statement each, so servers on one
//! database never both hold one. An upsert racing another waits on the row and then sees it held.

use std::time::Duration;

use deadpool_postgres::Pool;
use glave_ports::{BoxFuture, Holder, Lease, LeaseError, Leased, Leases, OrgId, ttl_millis};
use jiff::Timestamp;

use crate::store::{connection, database};

/// The database's now, in milliseconds since 1970: its transaction's start, the same throughout
/// one statement.
const NOW: &str = "(extract(epoch FROM now()) * 1000)::bigint";

/// The leases in the database.
#[derive(Clone)]
pub struct PostgresLeases {
    pool: Pool,
}

impl PostgresLeases {
    pub(crate) fn new(pool: Pool) -> Self {
        Self { pool }
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
        let client = connection(&self.pool).await.map_err(storage)?;
        let row = client
            .query_opt(
                &sql,
                &[
                    &org.as_uuid(),
                    &on.key(),
                    &holder.as_uuid(),
                    &ttl_millis(ttl),
                ],
            )
            .await
            .map_err(database(LeaseError::Storage))?;
        row.map(|row| {
            let expires: i64 = row.try_get(0).map_err(database(LeaseError::Storage))?;
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

impl Leases for PostgresLeases {
    fn acquire(
        &self,
        org: OrgId,
        on: Leased,
        holder: Holder,
        ttl: Duration,
    ) -> BoxFuture<'_, Result<Option<Lease>, LeaseError>> {
        let sql = format!(
            "INSERT INTO leases (org, key, holder, expires) VALUES ($1, $2, $3, {NOW} + $4)
             ON CONFLICT (org, key) DO UPDATE SET
                 holder = EXCLUDED.holder, expires = EXCLUDED.expires
                 WHERE leases.holder = EXCLUDED.holder OR leases.expires <= {NOW}
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
            "UPDATE leases SET expires = {NOW} + $4
             WHERE org = $1 AND key = $2 AND holder = $3
             RETURNING expires"
        );
        Box::pin(self.kept(sql, lease.org, lease.on, lease.holder, ttl))
    }

    fn release<'a>(&'a self, lease: &'a Lease) -> BoxFuture<'a, Result<(), LeaseError>> {
        Box::pin(async move {
            let client = connection(&self.pool).await.map_err(storage)?;
            client
                .execute(
                    "DELETE FROM leases WHERE org = $1 AND key = $2 AND holder = $3",
                    &[
                        &lease.org.as_uuid(),
                        &lease.on.key(),
                        &lease.holder.as_uuid(),
                    ],
                )
                .await
                .map(drop)
                .map_err(database(LeaseError::Storage))
        })
    }
}
