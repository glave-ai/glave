//! Sealing at rest on Postgres: the keys conformance suite, the at-rest suite on the tables'
//! own rows, and erasure. Skipped without `GLAVE_TEST_POSTGRES_URL`, like the other Postgres
//! tests.

use glave_event::{RawEvent, Seq, ThreadId};
use glave_keys::KeyId;
use glave_keys::conformance::event;
use glave_ports::conformance::{AtRest, KeptBlob, KeptPayload};
use glave_ports::{
    BlobStore, BoxFuture, EventLog, LogError, Objects as _, OrgId, Pieces, ProjectId, Scope,
    read_all as read_whole,
};
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

/// The tables' rows, read and written around the store, as the at-rest suite asks.
struct Rows<'a>(&'a Scratch);

impl Rows<'_> {
    async fn payload_now(&self, scope: Scope, thread: ThreadId, seq: Seq) -> KeptPayload {
        let row = self
            .0
            .admin
            .query_one(
                &format!(
                    "SELECT data, key_id FROM {}.events
                     WHERE org = $1 AND project = $2 AND thread = $3 AND seq = $4",
                    self.0.schema
                ),
                &[
                    &scope.org().as_uuid(),
                    &scope.project().as_uuid(),
                    &thread.as_uuid(),
                    &i64::try_from(seq.get()).unwrap(),
                ],
            )
            .await
            .unwrap();
        KeptPayload {
            bytes: row.get(0),
            key_id: row.get(1),
        }
    }

    async fn overwrite_now(&self, scope: Scope, thread: ThreadId, seq: Seq, kept: KeptPayload) {
        let written = self
            .0
            .admin
            .execute(
                &format!(
                    "UPDATE {}.events SET data = $1, key_id = $2
                     WHERE org = $3 AND project = $4 AND thread = $5 AND seq = $6",
                    self.0.schema
                ),
                &[
                    &kept.bytes,
                    &kept.key_id,
                    &scope.org().as_uuid(),
                    &scope.project().as_uuid(),
                    &thread.as_uuid(),
                    &i64::try_from(seq.get()).unwrap(),
                ],
            )
            .await
            .unwrap();
        assert_eq!(written, 1, "one row");
    }

    async fn blobs_now(&self, scope: Scope) -> Vec<KeptBlob> {
        let (org, project) = (scope.org().as_uuid(), scope.project().as_uuid());
        let schema = &self.0.schema;
        let rows = self
            .0
            .admin
            .query(
                &format!("SELECT name, object FROM {schema}.blobs WHERE org = $1 AND project = $2"),
                &[&org, &project],
            )
            .await
            .unwrap();
        let mut blobs = Vec::new();
        for row in rows {
            let name: String = row.get(0);
            let threads = self
                .0
                .admin
                .query(
                    &format!(
                        "SELECT thread FROM {schema}.blob_threads
                         WHERE org = $1 AND project = $2 AND name = $3"
                    ),
                    &[&org, &project, &name],
                )
                .await
                .unwrap()
                .iter()
                .map(|thread| ThreadId::try_from(thread.get::<_, Uuid>(0)).unwrap())
                .collect();
            let object: String = row.get(1);
            let mut bytes = self.0.objects.read(scope, &object).await.unwrap().unwrap();
            blobs.push(KeptBlob {
                name,
                bytes: read_whole(bytes.as_mut(), u64::MAX).await.unwrap(),
                threads,
            });
        }
        blobs
    }
}

impl AtRest for Rows<'_> {
    fn payload(
        &self,
        scope: Scope,
        thread: ThreadId,
        seq: Seq,
    ) -> BoxFuture<'_, Result<KeptPayload, String>> {
        Box::pin(async move { Ok(self.payload_now(scope, thread, seq).await) })
    }

    fn overwrite(
        &self,
        scope: Scope,
        thread: ThreadId,
        seq: Seq,
        kept: KeptPayload,
    ) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            self.overwrite_now(scope, thread, seq, kept).await;
            Ok(())
        })
    }

    fn blobs(&self, scope: Scope) -> BoxFuture<'_, Result<Vec<KeptBlob>, String>> {
        Box::pin(async move { Ok(self.blobs_now(scope).await) })
    }
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
fn the_tables_pass_the_at_rest_suite() {
    block_on!(async {
        let Some(scratch) = Scratch::new("at_rest").await else {
            return;
        };
        let store = scratch.store().await.unwrap();
        let rows = Rows(&scratch);
        let verdict = glave_ports::conformance::at_rest(&store.log(), &store.blobs(), &rows).await;
        drop(store);
        scratch.drop_schema().await;
        assert_eq!(verdict, Ok(()));
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
            .put(
                &here,
                written[0].thread,
                "t/p",
                1,
                Box::new(Pieces::from(b"x".to_vec())),
            )
            .await
            .unwrap();
        let kept = Rows(&scratch)
            .payload_now(other, written[0].thread, written[0].seq)
            .await;
        let october: KeyId = kept.key_id.unwrap().parse().unwrap();
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
