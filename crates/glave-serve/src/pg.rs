//! Postgres adapter for the accounts [`AccountStore`] port. Sync (r2d2 pool +
//! the `postgres` crate); `glave-serve` calls it from `spawn_blocking`, matching
//! the model-provider pattern. `migrate()` creates the tables idempotently — a
//! real migration tool comes later. Ids are TEXT (a stringified UUID) to keep the
//! type mapping trivial.

use anyhow::{Context, Result};
use glave_core::{
    personal_account_name, Account, AccountKind, AccountRole, AccountStore, Identity, Membership,
    User,
};
use postgres::NoTls;
use r2d2::Pool;
use r2d2_postgres::PostgresConnectionManager;
use sha2::{Digest, Sha256};

type PgPool = Pool<PostgresConnectionManager<NoTls>>;

/// Hash a secret (OTP code or session token) for storage. The DB only ever holds
/// the hash, so a leaked dump can't be replayed — the plaintext lives only in the
/// user's cookie (session) or inbox (OTP). SHA-256 is appropriate here: session
/// tokens are 128-bit random UUIDs, and OTP codes are short-lived + single-use.
fn hash_secret(secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    format!("{:x}", hasher.finalize())
}

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
             );
             CREATE TABLE IF NOT EXISTS accounts (
                 id                     TEXT PRIMARY KEY,
                 name                   TEXT NOT NULL,
                 kind                   TEXT NOT NULL DEFAULT 'personal',
                 verified_domains       TEXT[] NOT NULL DEFAULT '{}',
                 billing_contact_user_id TEXT REFERENCES users(id),
                 -- One personal account per user: the idempotency key for
                 -- ensure_personal_account. NULL for team accounts.
                 personal_owner_user_id TEXT UNIQUE REFERENCES users(id),
                 deleted_at             TIMESTAMPTZ
             );
             CREATE TABLE IF NOT EXISTS memberships (
                 account_id TEXT NOT NULL REFERENCES accounts(id),
                 user_id    TEXT NOT NULL REFERENCES users(id),
                 role       TEXT NOT NULL,
                 PRIMARY KEY (account_id, user_id)
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
        // The `code` column stores the HASH, not the plaintext code.
        conn.execute(
            "INSERT INTO otp_codes (email, code, expires_at)
             VALUES ($1, $2, NOW() + INTERVAL '10 minutes')",
            &[&email, &hash_secret(&code)],
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
                &[&email, &hash_secret(code)],
            )
            .context("verify otp")?;
        Ok(!rows.is_empty())
    }

    /// Create a 30-day session for a user; returns the opaque token.
    pub fn create_session(&self, user_id: &str) -> Result<String> {
        let token = uuid::Uuid::new_v4().to_string();
        let mut conn = self.pool.get().context("pg conn")?;
        // The `token` column stores the HASH; the plaintext lives only in the cookie.
        conn.execute(
            "INSERT INTO sessions (token, user_id, expires_at)
             VALUES ($1, $2, NOW() + INTERVAL '30 days')",
            &[&hash_secret(&token), &user_id],
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
                &[&hash_secret(token)],
            )
            .context("query session_user")?;
        Ok(row.map(|r| row_to_user(&r)))
    }

    /// End a session (logout).
    pub fn delete_session(&self, token: &str) -> Result<()> {
        let mut conn = self.pool.get().context("pg conn")?;
        conn.execute("DELETE FROM sessions WHERE token = $1", &[&hash_secret(token)])
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

/// Maps an `accounts` row selected as `(id, name, kind, verified_domains,
/// billing_contact_user_id)`. Unknown `kind` falls back to `Personal` (the safe,
/// least-privileged default).
fn row_to_account(row: &postgres::Row) -> Account {
    let kind = match row.get::<_, String>(2).as_str() {
        "team" => AccountKind::Team,
        _ => AccountKind::Personal,
    };
    Account {
        id: row.get(0),
        name: row.get(1),
        kind,
        verified_domains: row.get(3),
        billing_contact_user_id: row.get(4),
    }
}

/// Stored role string → [`AccountRole`]. Unknown values fall back to `Member` (the
/// least-privileged role).
fn role_from_str(role: &str) -> AccountRole {
    match role {
        "owner" => AccountRole::Owner,
        _ => AccountRole::Member,
    }
}

/// [`AccountRole`] → its stored string form.
fn role_to_str(role: AccountRole) -> &'static str {
    match role {
        AccountRole::Owner => "owner",
        AccountRole::Member => "member",
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

    fn create_user_with_identity(
        &self,
        email: &str,
        name: Option<&str>,
        provider: &str,
        subject: &str,
        email_verified: bool,
    ) -> Result<User> {
        let id = uuid::Uuid::new_v4().to_string();
        let mut conn = self.pool.get().context("pg conn")?;
        let mut tx = conn.transaction().context("begin tx")?;
        tx.execute(
            "INSERT INTO users (id, email, name) VALUES ($1, $2, $3)",
            &[&id, &email, &name],
        )
        .context("insert user")?;
        tx.execute(
            "INSERT INTO identities (provider, subject, email, email_verified, user_id)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (provider, subject) DO NOTHING",
            &[&provider, &subject, &email, &email_verified, &id],
        )
        .context("insert identity")?;
        tx.commit().context("commit new user")?;
        Ok(User {
            id,
            email: email.to_string(),
            name: name.map(str::to_string),
        })
    }

    fn ensure_personal_account(&self, owner: &User) -> Result<Account> {
        let mut conn = self.pool.get().context("pg conn")?;
        let mut tx = conn.transaction().context("begin tx")?;
        let id = uuid::Uuid::new_v4().to_string();
        let name = personal_account_name(owner);
        // Idempotent on personal_owner_user_id: a concurrent/duplicate first login
        // can't mint a second personal account.
        tx.execute(
            "INSERT INTO accounts (id, name, kind, billing_contact_user_id, personal_owner_user_id)
             VALUES ($1, $2, 'personal', $3, $3)
             ON CONFLICT (personal_owner_user_id) DO NOTHING",
            &[&id, &name, &owner.id],
        )
        .context("insert personal account")?;
        // Load the canonical row (existing or just-created).
        let row = tx
            .query_one(
                "SELECT id, name, kind, verified_domains, billing_contact_user_id
                 FROM accounts WHERE personal_owner_user_id = $1",
                &[&owner.id],
            )
            .context("load personal account")?;
        let account = row_to_account(&row);
        tx.execute(
            "INSERT INTO memberships (account_id, user_id, role)
             VALUES ($1, $2, 'owner')
             ON CONFLICT (account_id, user_id) DO NOTHING",
            &[&account.id, &owner.id],
        )
        .context("insert owner membership")?;
        tx.commit().context("commit personal account")?;
        Ok(account)
    }

    fn accounts_for_user(&self, user_id: &str) -> Result<Vec<(Account, AccountRole)>> {
        let mut conn = self.pool.get().context("pg conn")?;
        let rows = conn
            .query(
                "SELECT a.id, a.name, a.kind, a.verified_domains, a.billing_contact_user_id, m.role
                 FROM accounts a
                 JOIN memberships m ON m.account_id = a.id
                 WHERE m.user_id = $1 AND a.deleted_at IS NULL",
                &[&user_id],
            )
            .context("query accounts_for_user")?;
        let out = rows
            .iter()
            .map(|r| (row_to_account(r), role_from_str(&r.get::<_, String>(5))))
            .collect();
        Ok(out)
    }

    fn membership(&self, account_id: &str, user_id: &str) -> Result<Option<AccountRole>> {
        let mut conn = self.pool.get().context("pg conn")?;
        let row = conn
            .query_opt(
                "SELECT role FROM memberships WHERE account_id = $1 AND user_id = $2",
                &[&account_id, &user_id],
            )
            .context("query membership")?;
        Ok(row.map(|r| role_from_str(&r.get::<_, String>(0))))
    }

    fn add_membership(&self, membership: Membership) -> Result<()> {
        let role = role_to_str(membership.role);
        let mut conn = self.pool.get().context("pg conn")?;
        conn.execute(
            "INSERT INTO memberships (account_id, user_id, role)
             VALUES ($1, $2, $3)
             ON CONFLICT (account_id, user_id) DO NOTHING",
            &[&membership.account_id, &membership.user_id, &role],
        )
        .context("insert membership")?;
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
                "DROP TABLE IF EXISTS memberships; DROP TABLE IF EXISTS accounts; \
                 DROP TABLE IF EXISTS sessions; DROP TABLE IF EXISTS identities; \
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

        // The plaintext code must never be stored — only its hash.
        if let Ok(mut conn) = store.pool.get() {
            if let Ok(rows) = conn.query("SELECT code FROM otp_codes WHERE email = $1", &[&email]) {
                if let Some(row) = rows.first() {
                    let stored: String = row.get(0);
                    assert_ne!(stored, code, "OTP code is hashed at rest, not plaintext");
                    assert_eq!(stored, hash_secret(&code), "stored value is the SHA-256 hash");
                }
            }
        }

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

    #[test]
    fn pg_personal_account_is_provisioned_once() {
        let Some(store) = fresh_store() else {
            return;
        };
        let Ok(user) = resolve_login(&store, "email", "p@acme.com", "p@acme.com", true) else {
            return;
        };
        let first = store.ensure_personal_account(&user).ok().map(|a| a.id);
        let second = store.ensure_personal_account(&user).ok().map(|a| a.id);
        assert_eq!(first, second, "ensure is idempotent — one personal account");
        assert!(first.is_some());

        let account_id = first.unwrap_or_default();
        assert_eq!(
            store.membership(&account_id, &user.id).ok().flatten(),
            Some(AccountRole::Owner),
            "the creator is the sole Owner of their personal account"
        );
        assert_eq!(
            store.accounts_for_user(&user.id).ok().map(|v| v.len()),
            Some(1),
            "exactly one account membership"
        );
    }
}
