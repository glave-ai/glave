//! Postgres adapter for the accounts [`AccountStore`] port. Sync (r2d2 pool +
//! the `postgres` crate); `glave-serve` calls it from `spawn_blocking`, matching
//! the model-provider pattern. `migrate()` creates the tables idempotently — a
//! real migration tool comes later. Ids are TEXT (a stringified UUID) to keep the
//! type mapping trivial.

use anyhow::{Context, Result};
use glave_core::{AccountStore, Identity, User};
use postgres::NoTls;
use r2d2::Pool;
use r2d2_postgres::PostgresConnectionManager;

type PgPool = Pool<PostgresConnectionManager<NoTls>>;

pub struct PgAccountStore {
    pool: PgPool,
}

impl PgAccountStore {
    /// Connect and build a pool from a `postgres://…` URL.
    pub fn connect(database_url: &str) -> Result<Self> {
        let config: postgres::Config = database_url.parse().context("parsing DATABASE_URL")?;
        let manager = PostgresConnectionManager::new(config, NoTls);
        let pool = Pool::builder().build(manager).context("building pg pool")?;
        Ok(Self { pool })
    }

    /// Create the account tables if they don't exist (idempotent).
    pub fn migrate(&self) -> Result<()> {
        let mut conn = self.pool.get().context("pg conn")?;
        conn.batch_execute(
            "CREATE TABLE IF NOT EXISTS users (
                 id    TEXT PRIMARY KEY,
                 email TEXT NOT NULL,
                 name  TEXT
             );
             CREATE TABLE IF NOT EXISTS identities (
                 provider       TEXT NOT NULL,
                 subject        TEXT NOT NULL,
                 email          TEXT NOT NULL,
                 email_verified BOOLEAN NOT NULL,
                 user_id        TEXT NOT NULL REFERENCES users(id),
                 PRIMARY KEY (provider, subject)
             );
             CREATE INDEX IF NOT EXISTS identities_verified_email_idx
                 ON identities (email) WHERE email_verified;
             CREATE TABLE IF NOT EXISTS otp_codes (
                 email      TEXT NOT NULL,
                 code       TEXT NOT NULL,
                 expires_at TIMESTAMPTZ NOT NULL,
                 used       BOOLEAN NOT NULL DEFAULT FALSE
             );
             CREATE TABLE IF NOT EXISTS sessions (
                 token      TEXT PRIMARY KEY,
                 user_id    TEXT NOT NULL REFERENCES users(id),
                 expires_at TIMESTAMPTZ NOT NULL
             );",
        )
        .context("running migrations")?;
        Ok(())
    }
}

/// Passwordless login + sessions. Concrete (not on the `AccountStore` port) — this
/// is auth-flow state, used by the serve handlers. Expiry is handled in SQL
/// (`NOW()`), so no time crate. v0 stores the code/token as-is; hash them in prod.
impl PgAccountStore {
    /// Mint a 6-digit one-time code for `email` (valid 10 min). The caller delivers
    /// it (email in prod; logged in dev). Returned so dev can use it directly.
    pub fn issue_otp(&self, email: &str) -> Result<String> {
        let code = format!("{:06}", uuid::Uuid::new_v4().as_u128() % 1_000_000);
        let mut conn = self.pool.get().context("pg conn")?;
        conn.execute(
            "INSERT INTO otp_codes (email, code, expires_at)
             VALUES ($1, $2, NOW() + INTERVAL '10 minutes')",
            &[&email, &code],
        )
        .context("insert otp")?;
        Ok(code)
    }

    /// Consume a code: true iff it's the right code for `email`, unused, unexpired.
    /// Marks it used atomically, so it's strictly single-use.
    pub fn verify_otp(&self, email: &str, code: &str) -> Result<bool> {
        let mut conn = self.pool.get().context("pg conn")?;
        let rows = conn
            .query(
                "UPDATE otp_codes SET used = TRUE
                 WHERE email = $1 AND code = $2 AND used = FALSE AND expires_at > NOW()
                 RETURNING email",
                &[&email, &code],
            )
            .context("verify otp")?;
        Ok(!rows.is_empty())
    }

    /// Create a 30-day session for a user; returns the opaque token.
    pub fn create_session(&self, user_id: &str) -> Result<String> {
        let token = uuid::Uuid::new_v4().to_string();
        let mut conn = self.pool.get().context("pg conn")?;
        conn.execute(
            "INSERT INTO sessions (token, user_id, expires_at)
             VALUES ($1, $2, NOW() + INTERVAL '30 days')",
            &[&token, &user_id],
        )
        .context("insert session")?;
        Ok(token)
    }

    /// Resolve a session token to its user, if the session is live.
    pub fn session_user(&self, token: &str) -> Result<Option<User>> {
        let mut conn = self.pool.get().context("pg conn")?;
        let row = conn
            .query_opt(
                "SELECT u.id, u.email, u.name
                 FROM users u JOIN sessions s ON s.user_id = u.id
                 WHERE s.token = $1 AND s.expires_at > NOW()",
                &[&token],
            )
            .context("query session_user")?;
        Ok(row.map(|r| row_to_user(&r)))
    }

    /// End a session (logout).
    pub fn delete_session(&self, token: &str) -> Result<()> {
        let mut conn = self.pool.get().context("pg conn")?;
        conn.execute("DELETE FROM sessions WHERE token = $1", &[&token])
            .context("delete session")?;
        Ok(())
    }
}

fn row_to_user(row: &postgres::Row) -> User {
    User {
        id: row.get(0),
        email: row.get(1),
        name: row.get(2),
    }
}

impl AccountStore for PgAccountStore {
    fn user_by_identity(&self, provider: &str, subject: &str) -> Result<Option<User>> {
        let mut conn = self.pool.get().context("pg conn")?;
        let row = conn
            .query_opt(
                "SELECT u.id, u.email, u.name
                 FROM users u
                 JOIN identities i ON i.user_id = u.id
                 WHERE i.provider = $1 AND i.subject = $2",
                &[&provider, &subject],
            )
            .context("query user_by_identity")?;
        Ok(row.map(|r| row_to_user(&r)))
    }

    fn user_by_verified_email(&self, email: &str) -> Result<Option<User>> {
        let mut conn = self.pool.get().context("pg conn")?;
        let row = conn
            .query_opt(
                "SELECT u.id, u.email, u.name
                 FROM users u
                 WHERE u.email = $1
                   AND EXISTS (
                     SELECT 1 FROM identities i
                     WHERE i.email = $1 AND i.email_verified AND i.user_id = u.id
                   )
                 LIMIT 1",
                &[&email],
            )
            .context("query user_by_verified_email")?;
        Ok(row.map(|r| row_to_user(&r)))
    }

    fn create_user(&self, email: &str, name: Option<&str>) -> Result<User> {
        let id = uuid::Uuid::new_v4().to_string();
        let mut conn = self.pool.get().context("pg conn")?;
        conn.execute(
            "INSERT INTO users (id, email, name) VALUES ($1, $2, $3)",
            &[&id, &email, &name],
        )
        .context("insert user")?;
        Ok(User {
            id,
            email: email.to_string(),
            name: name.map(str::to_string),
        })
    }

    fn add_identity(&self, identity: Identity) -> Result<()> {
        let mut conn = self.pool.get().context("pg conn")?;
        conn.execute(
            "INSERT INTO identities (provider, subject, email, email_verified, user_id)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (provider, subject) DO NOTHING",
            &[
                &identity.provider,
                &identity.subject,
                &identity.email,
                &identity.email_verified,
                &identity.user_id,
            ],
        )
        .context("insert identity")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glave_core::resolve_login;

    // Runs only when GLAVE_TEST_PG points at a Postgres (e.g. via the dev compose).
    // Without it the test no-ops, so `cargo test` stays DB-free.
    fn fresh_store() -> Option<PgAccountStore> {
        let url = std::env::var("GLAVE_TEST_PG").ok()?;
        let store = PgAccountStore::connect(&url).ok()?;
        if let Ok(mut conn) = store.pool.get() {
            let _ = conn.batch_execute(
                "DROP TABLE IF EXISTS sessions; DROP TABLE IF EXISTS identities; \
                 DROP TABLE IF EXISTS otp_codes; DROP TABLE IF EXISTS users;",
            );
        }
        store.migrate().ok()?;
        Some(store)
    }

    #[test]
    fn pg_login_resolution_matches_the_domain_rules() {
        let Some(store) = fresh_store() else {
            return;
        };
        let first = resolve_login(&store, "google", "g-1", "a@acme.com", true);
        let second = resolve_login(&store, "google", "g-1", "a@acme.com", true);
        assert!(first.is_ok() && second.is_ok());
        assert_eq!(first.ok().map(|u| u.id), second.ok().map(|u| u.id));

        let linked = resolve_login(&store, "saml:acme", "s-1", "a@acme.com", true);
        assert!(linked.is_ok());

        let separate = resolve_login(&store, "github", "h-1", "a@acme.com", false);
        assert!(separate.is_ok());
        assert_ne!(
            linked.ok().map(|u| u.id),
            separate.ok().map(|u| u.id),
            "unverified email must not link to the existing user"
        );
    }

    #[test]
    fn pg_passwordless_flow_issues_verifies_and_sessions() {
        let Some(store) = fresh_store() else {
            return;
        };
        let email = "person@acme.com";

        let issued = store.issue_otp(email);
        assert!(issued.is_ok());
        let Ok(code) = issued else {
            return;
        };

        let wrong = if code == "000001" { "000002" } else { "000001" };
        assert_eq!(store.verify_otp(email, wrong).ok(), Some(false), "wrong code rejected");
        assert_eq!(store.verify_otp(email, &code).ok(), Some(true), "right code accepted");
        assert_eq!(
            store.verify_otp(email, &code).ok(),
            Some(false),
            "codes are single-use"
        );

        let user = resolve_login(&store, "email", email, email, true);
        assert!(user.is_ok());
        let Ok(user) = user else {
            return;
        };

        let token = store.create_session(&user.id);
        assert!(token.is_ok());
        let Ok(token) = token else {
            return;
        };

        assert_eq!(
            store.session_user(&token).ok().flatten().map(|u| u.id),
            Some(user.id.clone()),
            "live session resolves to the user"
        );
        assert!(store.delete_session(&token).is_ok());
        assert!(
            matches!(store.session_user(&token), Ok(None)),
            "deleted session resolves to nobody"
        );
    }
}
