//! Sealing at rest on Postgres: the keys conformance suite, what lands in the table,
//! tampering, and erasure. Skipped without
//! `GLAVE_TEST_POSTGRES_URL`, like the other Postgres tests.

use glave_event::RawEvent;
use glave_keys::KeyId;
use glave_keys::conformance::event;
use glave_ports::{BlobStore, EventLog, LogError, OrgId, ProjectId, Scope};
use glave_store_postgres::Store;
use uuid::Uuid;

use crate::support::{Scratch, block_on};

fn scope(n: u128) -> Scope {
    Scope::glaves_own(
        OrgId::from_uuid(Uuid::from_u128(1)),
        ProjectId::from_uuid(Uuid::from_u128(100 + n)),
    )
}

fn lines(events: &[RawEvent]) -> Vec<String> {
    events
        .iter()
        .map(|event| event.to_json().unwrap())
        .collect()
}

async fn read_all(
    store: &Store,
    scope: &Scope,
    event: &RawEvent,
) -> Result<Vec<RawEvent>, LogError> {
    store.log().read(scope, event.thread, None, 100).await
}

/// The stored `data` and `key_id` of one event.
async fn stored(scratch: &Scratch, scope: &Scope, event: &RawEvent) -> (Vec<u8>, Option<String>) {
    let row = scratch
            .admin
            .query_one(
                &format!(
                    "SELECT data, key_id FROM {}.events WHERE project = $1 AND thread = $2 AND seq = $3",
                    scratch.schema
                ),
                &[
                    &scope.project().as_uuid(),
                    &event.thread.as_uuid(),
                    &i64::try_from(event.seq.get()).unwrap(),
                ],
            )
            .await
            .unwrap();
    (row.get(0), row.get(1))
}

async fn overwrite(
    scratch: &Scratch,
    scope: &Scope,
    event: &RawEvent,
    row: &(Vec<u8>, Option<String>),
) {
    scratch
        .admin
        .execute(
            &format!(
                "UPDATE {}.events SET data = $1, key_id = $2
                     WHERE project = $3 AND thread = $4 AND seq = $5",
                scratch.schema
            ),
            &[
                &row.0,
                &row.1,
                &scope.project().as_uuid(),
                &event.thread.as_uuid(),
                &i64::try_from(event.seq.get()).unwrap(),
            ],
        )
        .await
        .unwrap();
}

#[test]
fn the_keys_pass_the_key_conformance_suite() {
    block_on!(async {
        let Some(scratch) = Scratch::new("keys").await else {
            return;
        };
        let store = scratch.store().await.unwrap();
        let verdict = glave_keys::conformance::keys(store.keys()).await;
        drop(store);
        scratch.drop_schema().await;
        assert_eq!(verdict, Ok(()));
    });
}

#[test]
fn payloads_are_sealed_and_a_moved_payload_does_not_open() {
    block_on!(async {
        let Some(scratch) = Scratch::new("moved").await else {
            return;
        };
        let store = scratch.store().await.unwrap();
        let (here, there) = (scope(1), scope(2));
        let a = [
            event(1, 1, "2026-10").unwrap(),
            event(1, 2, "2026-10").unwrap(),
        ];
        let b = [event(2, 1, "2026-10").unwrap()];
        store.log().append(&here, &a).await.unwrap();
        store.log().append(&here, &b).await.unwrap();
        store.log().append(&there, &a[..1]).await.unwrap();
        let first = stored(&scratch, &here, &a[0]).await;
        let sealed = first.1.clone().map(|id| id.parse::<KeyId>());
        let clear = String::from_utf8_lossy(&first.0).contains("completed");
        let mut moved = Vec::new();
        for (scope, target) in [(here, &a[1]), (here, &b[0]), (there, &a[0])] {
            let original = stored(&scratch, &scope, target).await;
            overwrite(&scratch, &scope, target, &first).await;
            moved.push(read_all(&store, &scope, target).await.is_err());
            overwrite(&scratch, &scope, target, &original).await;
        }
        let back = read_all(&store, &here, &a[0])
            .await
            .map(|events| lines(&events));
        drop(store);
        scratch.drop_schema().await;
        assert!(matches!(sealed, Some(Ok(_))), "{sealed:?}");
        assert!(!clear, "the payload is not stored in the clear");
        assert_eq!(moved, [true, true, true], "moved payloads must not open");
        assert_eq!(back.unwrap(), lines(&a));
    });
}

#[test]
fn erasure_makes_content_unreadable_and_keeps_the_rest() {
    block_on!(async {
        let Some(scratch) = Scratch::new("erase").await else {
            return;
        };
        let store = scratch.store().await.unwrap();
        let (here, other) = (scope(1), scope(2));
        let written = [
            event(1, 1, "2026-10").unwrap(),
            event(1, 2, "2026-11").unwrap(),
        ];
        store.log().append(&here, &written).await.unwrap();
        store.log().append(&other, &written).await.unwrap();
        let blob = store
            .blobs()
            .put(&here, written[0].thread, b"x".to_vec(), "t/p")
            .await
            .unwrap();
        let october: KeyId = stored(&scratch, &other, &written[0])
            .await
            .1
            .unwrap()
            .parse()
            .unwrap();
        store
            .keys()
            .destroy_data_key(&other, october)
            .await
            .unwrap();
        let month_gone = read_all(&store, &other, &written[0]).await.is_err();
        let november = store
            .log()
            .read(&other, written[0].thread, Some(written[0].seq), 10)
            .await
            .map(|events| lines(&events));
        store.keys().erase_project(&here).await.unwrap();
        let project_gone = read_all(&store, &here, &written[0]).await.is_err();
        let blob_gone = !matches!(store.blobs().get(&here, &blob.sha256).await, Ok(Some(_)));
        drop(store);
        scratch.drop_schema().await;
        assert!(month_gone, "a destroyed month does not open");
        assert_eq!(november.unwrap(), lines(&written[1..]));
        assert!(project_gone && blob_gone, "an erased project does not open");
    });
}
