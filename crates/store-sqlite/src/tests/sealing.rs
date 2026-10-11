//! Sealing at rest: what lands in the file, tampering, rows from before sealing, erasure.

use glave_event::RawEvent;
use glave_keys::KeyId;
use glave_keys::conformance::event;
use glave_ports::{BlobStore, LogError, OrgId, Scope, SpaceId, digest};
use rusqlite::{Connection, params};

use super::*;

fn scope(n: u128) -> Scope {
    Scope {
        org: OrgId::from_uuid(Uuid::from_u128(1)),
        space: SpaceId::from_uuid(Uuid::from_u128(100 + n)),
    }
}

/// A store on a fresh file, and a second, raw connection to the same file.
fn on_disk(name: &str) -> (PathBuf, Store, Connection) {
    let path = temp_db(name);
    let store = block_on(Store::open(&path, file_keys(&path))).unwrap();
    let raw = Connection::open(&path).unwrap();
    (path, store, raw)
}

fn events(thread: u64, seqs: &[(u64, &str)]) -> Vec<RawEvent> {
    seqs.iter()
        .map(|(seq, month)| event(thread, *seq, month).unwrap())
        .collect()
}

/// Events as their JSON lines, to compare them.
fn lines(events: &[RawEvent]) -> Vec<String> {
    events
        .iter()
        .map(|event| event.to_json().unwrap())
        .collect()
}

fn read_all(store: &Store, scope: &Scope, thread: &RawEvent) -> Result<Vec<RawEvent>, LogError> {
    block_on(store.log().read(scope, thread.thread, None, 100))
}

/// The stored `data` and `key_id` of one event.
fn stored(raw: &Connection, scope: &Scope, event: &RawEvent) -> (Vec<u8>, Option<String>) {
    raw.query_row(
        "SELECT data, key_id FROM events WHERE space = ?1 AND thread = ?2 AND seq = ?3",
        params![
            scope.space.to_string(),
            event.thread.to_string(),
            i64::try_from(event.seq.get()).unwrap()
        ],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap()
}

fn overwrite(raw: &Connection, scope: &Scope, event: &RawEvent, row: &(Vec<u8>, Option<String>)) {
    raw.execute(
        "UPDATE events SET data = ?1, key_id = ?2 WHERE space = ?3 AND thread = ?4 AND seq = ?5",
        params![
            row.0,
            row.1,
            scope.space.to_string(),
            event.thread.to_string(),
            i64::try_from(event.seq.get()).unwrap()
        ],
    )
    .unwrap();
}

#[test]
fn the_keys_pass_the_key_conformance_suite() {
    let store = Store::open_in_memory(keys()).unwrap();
    assert_eq!(
        block_on(glave_keys::conformance::keys(store.keys())),
        Ok(())
    );
}

#[test]
fn payloads_are_sealed_in_the_file_and_read_back_as_written() {
    let (path, store, raw) = on_disk("sealed");
    let here = scope(1);
    let written = events(1, &[(1, "2026-10"), (2, "2026-10")]);
    block_on(store.log().append(&here, &written)).unwrap();
    let (data, key_id) = stored(&raw, &here, &written[0]);
    let key_id: KeyId = key_id.expect("a new row is sealed").parse().unwrap();
    assert_eq!(
        (key_id.space, key_id.period.to_string()),
        (here.space, "2026-10".into())
    );
    assert!(!String::from_utf8_lossy(&data).contains("completed"));
    drop(store);
    let reopened = block_on(Store::open(&path, file_keys(&path))).unwrap();
    assert_eq!(
        lines(&read_all(&reopened, &here, &written[0]).unwrap()),
        lines(&written)
    );
}

#[test]
fn a_sealed_payload_moved_to_another_row_thread_or_space_does_not_open() {
    let (_, store, raw) = on_disk("moved");
    let (here, there) = (scope(1), scope(2));
    let a = events(1, &[(1, "2026-10"), (2, "2026-10")]);
    let b = events(2, &[(1, "2026-10")]);
    block_on(store.log().append(&here, &a)).unwrap();
    block_on(store.log().append(&here, &b)).unwrap();
    block_on(store.log().append(&there, &a[..1])).unwrap();
    let first = stored(&raw, &here, &a[0]);
    for (scope, target) in [(here, &a[1]), (here, &b[0]), (there, &a[0])] {
        let original = stored(&raw, &scope, target);
        overwrite(&raw, &scope, target, &first);
        let read = read_all(&store, &scope, target);
        assert!(read.is_err(), "moved into {scope} {target:?}: {read:?}");
        overwrite(&raw, &scope, target, &original);
        assert!(read_all(&store, &scope, target).is_ok());
    }
}

#[test]
fn rows_written_before_sealing_still_read_after_the_migration() {
    let path = temp_db("unsealed");
    let here = scope(1);
    let old = event(1, 1, "2026-09").unwrap();
    let sha = digest(b"old blob");
    {
        // A database as the first schema left it, with a row and a blob stored as written.
        let raw = Connection::open(&path).unwrap();
        raw.execute_batch(crate::schema::MIGRATIONS[0]).unwrap();
        raw.pragma_update(None, "user_version", 1).unwrap();
        raw.execute(
            "INSERT INTO events (org, space, thread, seq, id, at, type, v, glave, data)
             VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                here.org.to_string(),
                here.space.to_string(),
                old.thread.to_string(),
                old.id.to_string(),
                old.at.to_string(),
                old.kind.as_str(),
                old.v.get(),
                old.glave,
                old.data.get().as_bytes()
            ],
        )
        .unwrap();
        raw.execute(
            "INSERT INTO blobs (org, space, sha256, media_type, bytes)
             VALUES (?1, ?2, ?3, 'text/plain', ?4)",
            params![
                here.org.to_string(),
                here.space.to_string(),
                sha.as_str(),
                b"old blob".as_slice()
            ],
        )
        .unwrap();
    }
    let store = block_on(Store::open(&path, file_keys(&path))).unwrap();
    let raw = Connection::open(&path).unwrap();
    let new = event(1, 2, "2026-10").unwrap();
    block_on(store.log().append(&here, std::slice::from_ref(&new))).unwrap();
    assert_eq!(
        stored(&raw, &here, &old).1,
        None,
        "old rows stay as written"
    );
    assert!(
        stored(&raw, &here, &new).1.is_some(),
        "new writes are sealed"
    );
    assert_eq!(
        lines(&read_all(&store, &here, &old).unwrap()),
        lines(&[old, new])
    );
    let blob = block_on(store.blobs().get(&here, &sha)).unwrap();
    assert_eq!(blob.as_deref(), Some(b"old blob".as_slice()));
}

#[test]
fn erasing_a_space_makes_its_events_and_blobs_unreadable() {
    let (_, store, _raw) = on_disk("erase-space");
    let (here, other) = (scope(1), scope(2));
    let written = events(1, &[(1, "2026-10")]);
    block_on(store.log().append(&here, &written)).unwrap();
    block_on(store.log().append(&other, &written)).unwrap();
    let blob = block_on(
        store
            .blobs()
            .put(&here, written[0].thread, b"x".to_vec(), "t/p"),
    )
    .unwrap();
    block_on(store.keys().erase_space(&here)).unwrap();
    let read = read_all(&store, &here, &written[0]);
    assert!(
        matches!(&read, Err(LogError::Storage(message)) if message.contains("destroyed")),
        "{read:?}"
    );
    let got = block_on(store.blobs().get(&here, &blob.sha256));
    assert!(!matches!(got, Ok(Some(_))), "{got:?}");
    assert_eq!(
        lines(&read_all(&store, &other, &written[0]).unwrap()),
        lines(&written)
    );
}

#[test]
fn destroying_a_month_key_erases_that_month_and_keeps_the_rest() {
    let (_, store, raw) = on_disk("erase-month");
    let here = scope(1);
    let written = events(1, &[(1, "2026-10"), (2, "2026-11")]);
    block_on(store.log().append(&here, &written)).unwrap();
    let october: KeyId = stored(&raw, &here, &written[0]).1.unwrap().parse().unwrap();
    block_on(store.keys().destroy_data_key(&here, october)).unwrap();
    assert!(read_all(&store, &here, &written[0]).is_err());
    let november = block_on(
        store
            .log()
            .read(&here, written[0].thread, Some(written[0].seq), 10),
    );
    assert_eq!(lines(&november.unwrap()), lines(&written[1..]));
}

#[test]
fn blobs_are_stored_under_a_keyed_name_with_the_threads_that_use_them() {
    let (_, store, raw) = on_disk("blob-names");
    let here = scope(1);
    let [a, b] = [
        event(1, 1, "2026-10").unwrap(),
        event(2, 1, "2026-10").unwrap(),
    ];
    let reference = block_on(
        store
            .blobs()
            .put(&here, a.thread, b"shared".to_vec(), "t/p"),
    )
    .unwrap();
    block_on(
        store
            .blobs()
            .put(&here, b.thread, b"shared".to_vec(), "t/p"),
    )
    .unwrap();
    let (name, bytes): (String, Vec<u8>) = raw
        .query_row("SELECT name, bytes FROM blobs", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_ne!(name, reference.sha256.as_str(), "the hash is not stored");
    assert!(!String::from_utf8_lossy(&bytes).contains("shared"));
    let threads: i64 = raw
        .query_row(
            "SELECT COUNT(*) FROM blob_threads WHERE name = ?1",
            params![name],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(threads, 2);
    let got = block_on(store.blobs().get(&here, &reference.sha256)).unwrap();
    assert_eq!(got.as_deref(), Some(b"shared".as_slice()));
}

#[test]
fn another_root_cannot_read_the_file() {
    let (path, store, _raw) = on_disk("other-root");
    let here = scope(1);
    let written = events(1, &[(1, "2026-10")]);
    block_on(store.log().append(&here, &written)).unwrap();
    drop(store);
    let stranger = block_on(Store::open(&path, keys())).unwrap();
    assert!(read_all(&stranger, &here, &written[0]).is_err());
}
