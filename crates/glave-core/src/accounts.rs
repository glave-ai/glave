//! Accounts domain — the product's people. The database owns the user; auth
//! providers (password, OTP, social, and later SAML) are just [`Identity`] rows
//! that point at a [`User`]. Postgres is a future adapter behind [`AccountStore`];
//! offline tests use the in-memory store. These types are *identity*, not
//! credentials — auth is verified at the edge, never stored here.

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// A person. Source of truth for the user as the product knows them; all app data
/// foreign-keys to `id`. We own this row even when an external IdP verifies login.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
}

/// One way to authenticate, pointing at a [`User`]. `(provider, subject)` is the
/// authoritative per-identity key — e.g. `("google", google_sub)` or
/// `("saml:acme", name_id)`. `email_verified` gates cross-provider linking: never
/// link on an unverified email (that is an account-takeover hole).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub provider: String,
    pub subject: String,
    pub email: String,
    pub email_verified: bool,
    pub user_id: String,
}

/// A role within an org (B2B).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrgRole {
    Owner,
    Admin,
    Member,
}

/// An organization. Members join by invite or by a verified email domain
/// (domain capture) — the "company-wide" path without SSO.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Org {
    pub id: String,
    pub name: String,
    pub verified_domains: Vec<String>,
}

/// A user's place in an org.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Membership {
    pub org_id: String,
    pub user_id: String,
    pub role: OrgRole,
}

/// Port: persistence for accounts. Postgres implements this in production; the
/// in-memory store backs offline tests. The product depends only on this seam.
pub trait AccountStore {
    fn user_by_identity(&self, provider: &str, subject: &str) -> Result<Option<User>>;
    fn user_by_verified_email(&self, email: &str) -> Result<Option<User>>;
    fn create_user(&mut self, email: &str, name: Option<&str>) -> Result<User>;
    fn add_identity(&mut self, identity: Identity) -> Result<()>;
}

/// Resolve a successful authentication to a [`User`] — the one place login from
/// any provider converges:
/// 1. Known `(provider, subject)` → that user (a returning login).
/// 2. Else a **verified** email matching an existing user → link a new identity to
///    that user (the cross-provider / B2C→B2B bridge).
/// 3. Else → a brand-new user + identity.
///
/// Unverified emails never link — they always take path 3. That guard is what
/// stops "I made a Google account with your email" from hijacking your account.
pub fn resolve_login(
    store: &mut dyn AccountStore,
    provider: &str,
    subject: &str,
    email: &str,
    email_verified: bool,
) -> Result<User> {
    if let Some(user) = store.user_by_identity(provider, subject)? {
        return Ok(user);
    }
    let existing = if email_verified {
        store.user_by_verified_email(email)?
    } else {
        None
    };
    let user = match existing {
        Some(user) => user,
        None => store.create_user(email, None)?,
    };
    store.add_identity(Identity {
        provider: provider.to_string(),
        subject: subject.to_string(),
        email: email.to_string(),
        email_verified,
        user_id: user.id.clone(),
    })?;
    Ok(user)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct InMemoryAccounts {
        users: Vec<User>,
        identities: Vec<Identity>,
        next: usize,
    }

    impl AccountStore for InMemoryAccounts {
        fn user_by_identity(&self, provider: &str, subject: &str) -> Result<Option<User>> {
            let found = self
                .identities
                .iter()
                .find(|i| i.provider == provider && i.subject == subject)
                .and_then(|i| self.users.iter().find(|u| u.id == i.user_id))
                .cloned();
            Ok(found)
        }

        fn user_by_verified_email(&self, email: &str) -> Result<Option<User>> {
            let reachable = self
                .identities
                .iter()
                .any(|i| i.email == email && i.email_verified);
            if !reachable {
                return Ok(None);
            }
            Ok(self.users.iter().find(|u| u.email == email).cloned())
        }

        fn create_user(&mut self, email: &str, name: Option<&str>) -> Result<User> {
            self.next += 1;
            let user = User {
                id: format!("user-{}", self.next),
                email: email.to_string(),
                name: name.map(str::to_string),
            };
            self.users.push(user.clone());
            Ok(user)
        }

        fn add_identity(&mut self, identity: Identity) -> Result<()> {
            self.identities.push(identity);
            Ok(())
        }
    }

    #[test]
    fn returning_login_reuses_the_same_user() {
        let mut store = InMemoryAccounts::default();
        let first = resolve_login(&mut store, "google", "sub-1", "a@acme.com", true);
        let second = resolve_login(&mut store, "google", "sub-1", "a@acme.com", true);
        assert!(first.is_ok() && second.is_ok());
        assert_eq!(first.ok().map(|u| u.id), second.ok().map(|u| u.id));
        assert_eq!(store.users.len(), 1);
    }

    #[test]
    fn verified_email_links_a_new_provider_to_the_same_user() {
        let mut store = InMemoryAccounts::default();
        let _ = resolve_login(&mut store, "google", "g-1", "a@acme.com", true);
        let linked = resolve_login(&mut store, "saml:acme", "s-1", "a@acme.com", true);
        assert!(linked.is_ok());
        assert_eq!(store.users.len(), 1);
        assert_eq!(store.identities.len(), 2);
    }

    #[test]
    fn unverified_email_never_links() {
        let mut store = InMemoryAccounts::default();
        let _ = resolve_login(&mut store, "google", "g-1", "a@acme.com", true);
        let other = resolve_login(&mut store, "github", "h-1", "a@acme.com", false);
        assert!(other.is_ok());
        assert_eq!(store.users.len(), 2);
    }

    #[test]
    fn new_person_creates_a_user() {
        let mut store = InMemoryAccounts::default();
        let created = resolve_login(&mut store, "google", "g-9", "new@x.com", true);
        assert!(created.is_ok());
        assert_eq!(store.users.len(), 1);
    }
}
