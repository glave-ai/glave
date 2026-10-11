use std::sync::Arc;

use crate::conformance;
use crate::root::MemoryRoot;
use crate::storage::MemoryKeyStorage;

use super::*;

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

fn keys(cache_entries: usize) -> Keys<MemoryKeyStorage> {
    let mut settings = KeySettings::new(Arc::new(MemoryRoot::new().unwrap()));
    settings.cache_entries = cache_entries;
    settings.clock = Arc::new(|| "2026-10-10T12:00:00Z".parse().unwrap());
    Keys::new(settings, MemoryKeyStorage::new())
}

#[test]
fn memory_keys_pass_the_conformance_suite() {
    assert_eq!(block_on(conformance::keys(&keys(4096))), Ok(()));
}

#[test]
fn the_suite_passes_with_no_cache_so_every_key_is_unwrapped_from_storage() {
    assert_eq!(block_on(conformance::keys(&keys(0))), Ok(()));
}

#[test]
fn blobs_are_sealed_under_the_clocks_month() {
    let keys = keys(16);
    let scope = Scope {
        org: glave_ports::OrgId::from_uuid(uuid::Uuid::from_u128(1)),
        space: glave_ports::SpaceId::from_uuid(uuid::Uuid::from_u128(2)),
    };
    let sha = glave_ports::digest(b"x");
    let (_, sealed) = block_on(keys.seal_blob(&scope, &sha, b"x")).unwrap();
    let id: KeyId = sealed.key_id.parse().unwrap();
    assert_eq!(id.period.to_string(), "2026-10");
}

#[test]
fn another_root_cannot_open_what_this_one_sealed() {
    let storage = MemoryKeyStorage::new();
    let scope = Scope {
        org: glave_ports::OrgId::from_uuid(uuid::Uuid::from_u128(1)),
        space: glave_ports::SpaceId::from_uuid(uuid::Uuid::from_u128(2)),
    };
    let event = conformance::event(1, 1, "2026-10").unwrap();
    let first = Keys::new(
        KeySettings::new(Arc::new(MemoryRoot::new().unwrap())),
        storage,
    );
    let sealed = block_on(first.seal_event(&scope, &event)).unwrap();
    let second = Keys::new(
        KeySettings::new(Arc::new(MemoryRoot::new().unwrap())),
        first.storage,
    );
    let opened = block_on(second.open_event(
        &scope,
        &EventFields::from(&event),
        &sealed.key_id,
        &sealed.bytes,
    ));
    assert!(matches!(opened, Err(KeyError::Tampered(_))), "{opened:?}");
}
