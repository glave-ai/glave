//! What the sign-in tests share: a store of their own in memory, and the scope they keep their
//! ledgers in.

use std::sync::Arc;

use glave_ports::{OrgId, ProjectId, Scope};
use glave_store_sqlite::Store;
use uuid::Uuid;

/// A store of its own, in memory.
pub(crate) fn store() -> Store {
    let root = glave_keys::InMemoryRoot::new().unwrap();
    Store::open_in_memory(glave_keys::KeySettings::new(Arc::new(root))).unwrap()
}

/// The scope the tests' ledgers are kept in.
pub(crate) fn scope() -> Scope {
    Scope::glaves_own(
        OrgId::from_uuid(Uuid::from_u128(1)),
        ProjectId::from_uuid(Uuid::from_u128(2)),
    )
}
