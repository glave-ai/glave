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

/// A member's role within an [`Account`]. `Owner` controls billing + membership;
/// `Member` uses the account's resources. (`Admin` — a non-billing manager — is
/// deferred until a real team needs one; the enum widens cheaply later.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountRole {
    Owner,
    Member,
}

/// Whether an [`Account`] is one person's private space or a shared team. This is a
/// **cosmetic** flag for UI/onboarding only — authorization, billing, and seat
/// logic never branch on it, so a one-member team behaves exactly like a personal
/// account. Personal→team is therefore a non-migration: flip `kind`, invite members.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountKind {
    Personal,
    Team,
}

/// The single resource-owning + billing entity. Every app resource foreign-keys to
/// an `Account` (never directly to a [`User`]); a personal account is just an
/// account with one `Owner` membership. `billing_contact_user_id` is the one human
/// who pays — distinct from the (possibly many) `Owner`s. Team members join by
/// invite or a verified email domain (domain capture) — company-wide without SSO.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub kind: AccountKind,
    pub verified_domains: Vec<String>,
    pub billing_contact_user_id: Option<String>,
}

/// A user's place in an [`Account`] — the access edge. The presence of the row is
/// the authorization fact; `role` gates governance actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Membership {
    pub account_id: String,
    pub user_id: String,
    pub role: AccountRole,
}

/// A friendly default name for a freshly provisioned personal account: the user's
/// name if known, else the email's local part, else a neutral fallback.
pub fn personal_account_name(user: &User) -> String {
    if let Some(name) = user.name.as_deref() {
        if !name.is_empty() {
            return name.to_string();
        }
    }
    user.email
        .split('@')
        .next()
        .filter(|local| !local.is_empty())
        .unwrap_or("Personal")
        .to_string()
}

/// Port: persistence for accounts. Methods take `&self` — the *database* is the
/// mutable state, not the handle — so a single store can be shared (`Arc`) across
/// concurrent requests. Postgres implements this in production; the in-memory
/// store backs offline tests.
pub trait AccountStore {
    fn user_by_identity(&self, provider: &str, subject: &str) -> Result<Option<User>>;
    fn user_by_verified_email(&self, email: &str) -> Result<Option<User>>;
    fn create_user(&self, email: &str, name: Option<&str>) -> Result<User>;
    fn add_identity(&self, identity: Identity) -> Result<()>;

    /// Return the user's personal [`Account`], creating it (plus the sole `Owner`
    /// membership, with the user as billing contact) on first call. **Idempotent**:
    /// one human keeps exactly one personal account, so a double-submitted login or
    /// a concurrent web+CLI first-login can never mint a second. The adapter does
    /// the create atomically. Called from the auth handler — `resolve_login` stays
    /// a pure identity-resolution function.
    fn ensure_personal_account(&self, owner: &User) -> Result<Account>;

    /// Every account the user is a member of, with their role — for the account
    /// switcher and access checks.
    fn accounts_for_user(&self, user_id: &str) -> Result<Vec<(Account, AccountRole)>>;

    /// The user's role in an account, or `None` if they are not a member. This is
    /// the authorization check: no row → no access.
    fn membership(&self, account_id: &str, user_id: &str) -> Result<Option<AccountRole>>;

    /// Add a user to an account (idempotent on `(account_id, user_id)`). Used by
    /// invites and domain capture later.
    fn add_membership(&self, membership: Membership) -> Result<()>;
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
    store: &dyn AccountStore,
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
    use std::cell::RefCell;

    #[derive(Default)]
    struct InMemoryAccounts {
        users: RefCell<Vec<User>>,
        identities: RefCell<Vec<Identity>>,
        accounts: RefCell<Vec<Account>>,
        memberships: RefCell<Vec<Membership>>,
        next: RefCell<usize>,
    }

    impl InMemoryAccounts {
        /// The personal account this user owns, if any — the idempotency key for
        /// `ensure_personal_account`.
        fn personal_account_for(&self, user_id: &str) -> Option<Account> {
            let memberships = self.memberships.borrow();
            let accounts = self.accounts.borrow();
            memberships
                .iter()
                .filter(|m| m.user_id == user_id && m.role == AccountRole::Owner)
                .filter_map(|m| accounts.iter().find(|a| a.id == m.account_id))
                .find(|a| a.kind == AccountKind::Personal)
                .cloned()
        }
    }

    impl AccountStore for InMemoryAccounts {
        fn user_by_identity(&self, provider: &str, subject: &str) -> Result<Option<User>> {
            let identities = self.identities.borrow();
            let users = self.users.borrow();
            let found = identities
                .iter()
                .find(|i| i.provider == provider && i.subject == subject)
                .and_then(|i| users.iter().find(|u| u.id == i.user_id))
                .cloned();
            Ok(found)
        }

        fn user_by_verified_email(&self, email: &str) -> Result<Option<User>> {
            let identities = self.identities.borrow();
            let reachable = identities
                .iter()
                .any(|i| i.email == email && i.email_verified);
            if !reachable {
                return Ok(None);
            }
            Ok(self.users.borrow().iter().find(|u| u.email == email).cloned())
        }

        fn create_user(&self, email: &str, name: Option<&str>) -> Result<User> {
            let mut next = self.next.borrow_mut();
            *next += 1;
            let user = User {
                id: format!("user-{}", *next),
                email: email.to_string(),
                name: name.map(str::to_string),
            };
            self.users.borrow_mut().push(user.clone());
            Ok(user)
        }

        fn add_identity(&self, identity: Identity) -> Result<()> {
            self.identities.borrow_mut().push(identity);
            Ok(())
        }

        fn ensure_personal_account(&self, owner: &User) -> Result<Account> {
            if let Some(existing) = self.personal_account_for(&owner.id) {
                return Ok(existing);
            }
            let mut next = self.next.borrow_mut();
            *next += 1;
            let account = Account {
                id: format!("acct-{}", *next),
                name: personal_account_name(owner),
                kind: AccountKind::Personal,
                verified_domains: Vec::new(),
                billing_contact_user_id: Some(owner.id.clone()),
            };
            self.accounts.borrow_mut().push(account.clone());
            self.memberships.borrow_mut().push(Membership {
                account_id: account.id.clone(),
                user_id: owner.id.clone(),
                role: AccountRole::Owner,
            });
            Ok(account)
        }

        fn accounts_for_user(&self, user_id: &str) -> Result<Vec<(Account, AccountRole)>> {
            let memberships = self.memberships.borrow();
            let accounts = self.accounts.borrow();
            let out = memberships
                .iter()
                .filter(|m| m.user_id == user_id)
                .filter_map(|m| {
                    accounts
                        .iter()
                        .find(|a| a.id == m.account_id)
                        .map(|a| (a.clone(), m.role))
                })
                .collect();
            Ok(out)
        }

        fn membership(&self, account_id: &str, user_id: &str) -> Result<Option<AccountRole>> {
            let memberships = self.memberships.borrow();
            Ok(memberships
                .iter()
                .find(|m| m.account_id == account_id && m.user_id == user_id)
                .map(|m| m.role))
        }

        fn add_membership(&self, membership: Membership) -> Result<()> {
            let mut memberships = self.memberships.borrow_mut();
            let exists = memberships
                .iter()
                .any(|m| m.account_id == membership.account_id && m.user_id == membership.user_id);
            if !exists {
                memberships.push(membership);
            }
            Ok(())
        }
    }

    #[test]
    fn returning_login_reuses_the_same_user() {
        let store = InMemoryAccounts::default();
        let first = resolve_login(&store, "google", "sub-1", "a@acme.com", true);
        let second = resolve_login(&store, "google", "sub-1", "a@acme.com", true);
        assert!(first.is_ok() && second.is_ok());
        assert_eq!(first.ok().map(|u| u.id), second.ok().map(|u| u.id));
        assert_eq!(store.users.borrow().len(), 1);
    }

    #[test]
    fn verified_email_links_a_new_provider_to_the_same_user() {
        let store = InMemoryAccounts::default();
        let _ = resolve_login(&store, "google", "g-1", "a@acme.com", true);
        let linked = resolve_login(&store, "saml:acme", "s-1", "a@acme.com", true);
        assert!(linked.is_ok());
        assert_eq!(store.users.borrow().len(), 1);
        assert_eq!(store.identities.borrow().len(), 2);
    }

    #[test]
    fn unverified_email_never_links() {
        let store = InMemoryAccounts::default();
        let _ = resolve_login(&store, "google", "g-1", "a@acme.com", true);
        let other = resolve_login(&store, "github", "h-1", "a@acme.com", false);
        assert!(other.is_ok());
        assert_eq!(store.users.borrow().len(), 2);
    }

    #[test]
    fn new_person_creates_a_user() {
        let store = InMemoryAccounts::default();
        let created = resolve_login(&store, "google", "g-9", "new@x.com", true);
        assert!(created.is_ok());
        assert_eq!(store.users.borrow().len(), 1);
    }

    #[test]
    fn first_login_provisions_one_personal_account() {
        let store = InMemoryAccounts::default();
        let Ok(user) = resolve_login(&store, "google", "g-1", "a@acme.com", true) else {
            return;
        };
        let Ok(account) = store.ensure_personal_account(&user) else {
            return;
        };
        assert_eq!(account.kind, AccountKind::Personal);
        assert_eq!(
            account.billing_contact_user_id.as_deref(),
            Some(user.id.as_str()),
            "the creator is the billing contact"
        );
        assert_eq!(
            store.membership(&account.id, &user.id).ok().flatten(),
            Some(AccountRole::Owner),
            "the creator is the sole Owner"
        );
        assert_eq!(store.accounts.borrow().len(), 1);
        assert_eq!(store.memberships.borrow().len(), 1);
    }

    #[test]
    fn ensure_personal_account_is_idempotent() {
        let store = InMemoryAccounts::default();
        let Ok(user) = resolve_login(&store, "google", "g-1", "a@acme.com", true) else {
            return;
        };
        let first = store.ensure_personal_account(&user).ok().map(|a| a.id);
        let second = store.ensure_personal_account(&user).ok().map(|a| a.id);
        assert_eq!(first, second, "same personal account on the second call");
        assert!(first.is_some());
        assert_eq!(store.accounts.borrow().len(), 1, "no second account is minted");
    }

    #[test]
    fn linked_provider_keeps_one_personal_account() {
        let store = InMemoryAccounts::default();
        let Ok(first) = resolve_login(&store, "google", "g-1", "a@acme.com", true) else {
            return;
        };
        let _ = store.ensure_personal_account(&first);
        let Ok(linked) = resolve_login(&store, "saml:acme", "s-1", "a@acme.com", true) else {
            return;
        };
        let _ = store.ensure_personal_account(&linked);
        assert_eq!(first.id, linked.id, "verified email links to the same user");
        assert_eq!(
            store.accounts.borrow().len(),
            1,
            "one human keeps one personal account across login methods"
        );
    }
}
