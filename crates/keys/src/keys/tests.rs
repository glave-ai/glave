use std::sync::Arc;

use glave_ports::block_on;
use glave_ports::testing::ManualClock;

use crate::conformance;
use crate::root::InMemoryRoot;
use crate::storage::InMemoryKeyStorage;

use super::*;

fn keys(cache_entries: usize) -> Keys<InMemoryKeyStorage> {
    let mut settings = KeySettings::new(Arc::new(InMemoryRoot::new().unwrap()));
    settings.cache_entries = cache_entries;
    let at: jiff::Timestamp = "2026-10-10T12:00:00Z".parse().unwrap();
    settings.clock = Arc::new(ManualClock::at_millis(at.as_millisecond()));
    Keys::new(settings, InMemoryKeyStorage::new())
}

#[test]
fn in_memory_keys_pass_the_conformance_suite() {
    assert_eq!(block_on!(conformance::keys(&keys(4096))), Ok(()));
}

#[test]
fn the_suite_passes_with_no_cache_so_every_key_is_unwrapped_from_storage() {
    assert_eq!(block_on!(conformance::keys(&keys(0))), Ok(()));
}

#[test]
fn another_root_cannot_open_what_this_one_sealed() {
    let storage = InMemoryKeyStorage::new();
    let scope = Scope::glaves_own(
        glave_ports::OrgId::from_uuid(uuid::Uuid::from_u128(1)),
        glave_ports::ProjectId::from_uuid(uuid::Uuid::from_u128(2)),
    );
    let event = conformance::event(1, 1, "2026-10").unwrap();
    let first = Keys::new(
        KeySettings::new(Arc::new(InMemoryRoot::new().unwrap())),
        storage,
    );
    let sealed = block_on!(first.seal_event(&scope, &event)).unwrap();
    let second = Keys::new(
        KeySettings::new(Arc::new(InMemoryRoot::new().unwrap())),
        first.storage,
    );
    let opened = block_on!(second.open_event(
        &scope,
        &EventFields::from(&event),
        &sealed.key_id,
        &sealed.bytes,
    ));
    assert!(matches!(opened, Err(KeyError::Tampered(_))), "{opened:?}");
}

#[test]
fn search_terms_are_keyed_per_project_and_stable() {
    let keys = keys(16);
    let scope = |project| {
        Scope::glaves_own(
            glave_ports::OrgId::from_uuid(uuid::Uuid::from_u128(1)),
            glave_ports::ProjectId::from_uuid(uuid::Uuid::from_u128(project)),
        )
    };
    let words = vec!["deploy".to_owned(), "render".to_owned()];
    assert_eq!(
        block_on!(keys.search_terms(&scope(2), &words)).unwrap(),
        None,
        "a project with no key finds nothing"
    );
    let event = conformance::event(1, 1, "2026-10").unwrap();
    block_on!(keys.seal_event(&scope(2), &event)).unwrap();
    block_on!(keys.seal_event(&scope(3), &event)).unwrap();
    let here = block_on!(keys.search_terms(&scope(2), &words))
        .unwrap()
        .unwrap();
    assert_eq!(here.len(), 2);
    assert_ne!(here.first(), here.get(1));
    assert_eq!(
        block_on!(keys.search_terms(&scope(2), &words))
            .unwrap()
            .unwrap(),
        here
    );
    let there = block_on!(keys.search_terms(&scope(3), &words))
        .unwrap()
        .unwrap();
    assert_ne!(there, here, "another project's words never match");
}

#[test]
fn sealed_terms_open_only_in_their_own_row() {
    let keys = keys(16);
    let scope = Scope::glaves_own(
        glave_ports::OrgId::from_uuid(uuid::Uuid::from_u128(1)),
        glave_ports::ProjectId::from_uuid(uuid::Uuid::from_u128(2)),
    );
    let event = conformance::event(1, 1, "2026-10").unwrap();
    let words = vec!["deploy".to_owned(), "render".to_owned()];
    assert_eq!(
        block_on!(keys.seal_terms(&scope, &event, &words)).unwrap(),
        None,
        "a project with no key indexes nothing"
    );
    block_on!(keys.seal_event(&scope, &event)).unwrap();
    let sealed = block_on!(keys.seal_terms(&scope, &event, &words))
        .unwrap()
        .unwrap();
    let hashed = block_on!(keys.search_terms(&scope, &words))
        .unwrap()
        .unwrap();
    let found = |seq| glave_ports::search::Found {
        thread: event.thread,
        seq: Seq::new(seq).unwrap(),
    };
    let open =
        |seq| block_on!(keys.open_row(&scope, (7, found(seq)), &sealed.key_id, &sealed.bytes));
    let opened = open(1).unwrap();
    assert_eq!(
        (opened.pos, opened.found, &opened.terms),
        (7, found(1), &hashed)
    );
    assert_eq!(opened.sealed, sealed.bytes.len() as u64);
    let moved = open(2);
    assert!(
        moved.is_err(),
        "a row moved to another event does not open: {moved:?}"
    );
    assert!(
        !sealed
            .bytes
            .windows(8)
            .any(|w| hashed.iter().any(|t| w == t.to_be_bytes())),
        "no hashed word shows in the sealed row"
    );
    let id: KeyId = sealed.key_id.parse().unwrap();
    block_on!(keys.destroy_data_key(&scope, id)).unwrap();
    assert_eq!(
        open(1).unwrap().terms,
        [0u64; 0],
        "an erased month's words are gone"
    );
}
