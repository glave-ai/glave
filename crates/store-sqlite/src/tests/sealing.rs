//! Sealing at rest: the at-rest suite on the file's own rows, the key file, erasure.

use std::sync::Mutex;

use glave_event::{RawEvent, Seq, ThreadId};
use glave_keys::KeyId;
use glave_keys::conformance::event;
use glave_ports::conformance::{AtRest, KeptBlob, KeptPayload};
use glave_ports::testing::TempDir;
use glave_ports::{BlobStore, BoxFuture, LogError, OrgId, Pieces, ProjectId, Scope};
use rusqlite::{Connection, params};

use super::*;

fn scope(n: u128) -> Scope {
    Scope::glaves_own(
        OrgId::from_uuid(Uuid::from_u128(1)),
        ProjectId::from_uuid(Uuid::from_u128(100 + n)),
    )
}

/// A store on a fresh file, its rows read straight from the same file, and its folder,
/// removed when that drops.
fn on_disk(name: &str) -> (PathBuf, Store, Rows, TempDir) {
    let (dir, path) = temp_db(name);
    let store = block_on!(Store::open(&path, file_keys(&path))).unwrap();
    let raw = Connection::open(&path).unwrap();
    (path, store, Rows(Mutex::new(raw)), dir)
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
    block_on!(store.log().read(scope, thread.thread, None, 100))
}

/// The file's rows, read and written around the store, as the at-rest suite asks.
struct Rows(Mutex<Connection>);

impl Rows {
    fn payload_now(&self, scope: Scope, thread: ThreadId, seq: Seq) -> KeptPayload {
        let found = self.0.lock().unwrap().query_row(
            "SELECT data, key_id FROM events
             WHERE org = ?1 AND project = ?2 AND thread = ?3 AND seq = ?4",
            params![
                scope.org().to_string(),
                scope.project().to_string(),
                thread.to_string(),
                i64::try_from(seq.get()).unwrap()
            ],
            |row| {
                Ok(KeptPayload {
                    bytes: row.get(0)?,
                    key_id: row.get(1)?,
                })
            },
        );
        found.unwrap()
    }

    fn overwrite_now(&self, scope: Scope, thread: ThreadId, seq: Seq, kept: &KeptPayload) {
        let written = self.0.lock().unwrap().execute(
            "UPDATE events SET data = ?1, key_id = ?2
             WHERE org = ?3 AND project = ?4 AND thread = ?5 AND seq = ?6",
            params![
                kept.bytes,
                kept.key_id,
                scope.org().to_string(),
                scope.project().to_string(),
                thread.to_string(),
                i64::try_from(seq.get()).unwrap()
            ],
        );
        assert_eq!(written.unwrap(), 1, "one row");
    }

    fn blobs_now(&self, scope: Scope) -> Vec<KeptBlob> {
        let raw = self.0.lock().unwrap();
        let (org, project) = (scope.org().to_string(), scope.project().to_string());
        let records: Vec<(String, String)> = raw
            .prepare("SELECT name, object FROM blobs WHERE org = ?1 AND project = ?2")
            .unwrap()
            .query_map(params![org, project], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let mut kept = Vec::new();
        for (name, object) in records {
            let chunks: Vec<Vec<u8>> = raw
                .prepare(
                    "SELECT bytes FROM blob_chunks WHERE org = ?1 AND project = ?2 AND object = ?3
                     ORDER BY n",
                )
                .unwrap()
                .query_map(params![org, project, object], |row| row.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            kept.push((name, chunks.concat()));
        }
        let mut blobs = Vec::new();
        for (name, bytes) in kept {
            let threads: Vec<String> = raw
                .prepare(
                    "SELECT thread FROM blob_threads WHERE org = ?1 AND project = ?2 AND name = ?3",
                )
                .unwrap()
                .query_map(params![org, project, name], |row| row.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            let threads = threads
                .iter()
                .map(|thread| ThreadId::try_from(Uuid::parse_str(thread).unwrap()).unwrap())
                .collect();
            blobs.push(KeptBlob {
                name,
                bytes,
                threads,
            });
        }
        blobs
    }

    /// The key that sealed `event`'s payload in `scope`.
    fn key_of(&self, scope: &Scope, event: &RawEvent) -> KeyId {
        let kept = self.payload_now(*scope, event.thread, event.seq);
        kept.key_id.unwrap().parse().unwrap()
    }
}

impl AtRest for Rows {
    fn payload(
        &self,
        scope: Scope,
        thread: ThreadId,
        seq: Seq,
    ) -> BoxFuture<'_, Result<KeptPayload, String>> {
        Box::pin(std::future::ready(Ok(self.payload_now(scope, thread, seq))))
    }

    fn overwrite(
        &self,
        scope: Scope,
        thread: ThreadId,
        seq: Seq,
        kept: KeptPayload,
    ) -> BoxFuture<'_, Result<(), String>> {
        self.overwrite_now(scope, thread, seq, &kept);
        Box::pin(std::future::ready(Ok(())))
    }

    fn blobs(&self, scope: Scope) -> BoxFuture<'_, Result<Vec<KeptBlob>, String>> {
        Box::pin(std::future::ready(Ok(self.blobs_now(scope))))
    }
}

#[test]
fn the_keys_pass_the_key_conformance_suite() {
    let store = Store::open_in_memory(keys()).unwrap();
    assert_eq!(
        block_on!(glave_keys::conformance::keys(store.keys())),
        Ok(())
    );
}

#[test]
fn the_file_passes_the_at_rest_suite() {
    let (_, store, rows, _dir) = on_disk("at-rest");
    let verdict = block_on!(conformance::at_rest(&store.log(), &store.blobs(), &rows));
    assert_eq!(verdict, Ok(()));
}

#[test]
fn payloads_are_sealed_by_month_and_read_back_after_reopening() {
    let (path, store, rows, _dir) = on_disk("sealed");
    let here = scope(1);
    let written = events(1, &[(1, "2026-10"), (2, "2026-10")]);
    block_on!(store.log().append(&here, &written)).unwrap();
    let key_id = rows.key_of(&here, &written[0]);
    assert_eq!(
        (key_id.project, key_id.period.to_string()),
        (here.project(), "2026-10".into())
    );
    drop(store);
    let reopened = block_on!(Store::open(&path, file_keys(&path))).unwrap();
    assert_eq!(
        lines(&read_all(&reopened, &here, &written[0]).unwrap()),
        lines(&written)
    );
}

#[test]
fn erasing_a_project_makes_its_events_and_blobs_unreadable() {
    let (_, store, _rows, _dir) = on_disk("erase-project");
    let (here, other) = (scope(1), scope(2));
    let written = events(1, &[(1, "2026-10")]);
    block_on!(store.log().append(&here, &written)).unwrap();
    block_on!(store.log().append(&other, &written)).unwrap();
    let blob = block_on!(store.blobs().put(
        &here,
        written[0].thread,
        "t/p",
        1,
        Box::new(Pieces::from(b"x".to_vec()))
    ),)
    .unwrap();
    block_on!(store.keys().erase_project(&here)).unwrap();
    let read = read_all(&store, &here, &written[0]);
    assert!(
        matches!(&read, Err(LogError::Storage(message)) if message.contains("destroyed")),
        "{read:?}"
    );
    let got = block_on!(store.blobs().get(&here, &blob.sha256));
    assert!(!matches!(got, Ok(Some(_))), "{got:?}");
    assert_eq!(
        lines(&read_all(&store, &other, &written[0]).unwrap()),
        lines(&written)
    );
}

#[test]
fn destroying_a_month_key_erases_that_month_and_keeps_the_rest() {
    let (_, store, rows, _dir) = on_disk("erase-month");
    let here = scope(1);
    let written = events(1, &[(1, "2026-10"), (2, "2026-11")]);
    block_on!(store.log().append(&here, &written)).unwrap();
    let october = rows.key_of(&here, &written[0]);
    block_on!(store.keys().destroy_data_key(&here, october)).unwrap();
    assert!(read_all(&store, &here, &written[0]).is_err());
    let november = block_on!(
        store
            .log()
            .read(&here, written[0].thread, Some(written[0].seq), 10),
    );
    assert_eq!(lines(&november.unwrap()), lines(&written[1..]));
}

#[test]
fn another_root_cannot_read_the_file() {
    let (path, store, _rows, _dir) = on_disk("other-root");
    let here = scope(1);
    let written = events(1, &[(1, "2026-10")]);
    block_on!(store.log().append(&here, &written)).unwrap();
    drop(store);
    let stranger = block_on!(Store::open(&path, keys())).unwrap();
    assert!(read_all(&stranger, &here, &written[0]).is_err());
}
