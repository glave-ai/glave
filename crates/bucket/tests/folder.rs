//! A folder as the bucket: it passes the objects suite, and the blob store suite under the local
//! database; it holds only sealed bytes, under random names in a folder per organization and
//! project; and what a write abandoned or a release deleted leaves nothing behind.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glave_bucket::Bucket;
use glave_event::ThreadId;
use glave_keys::{InMemoryRoot, KeySettings};
use glave_ports::testing::TempDir;
use glave_ports::{
    BlobError, BlobStore as _, OrgId, Pieces, ProjectId, Scope, block_on, conformance, digest,
};
use glave_store_sqlite::Store;
use uuid::Uuid;

/// A fresh folder for the bucket, and a database beside it.
fn places(name: &str) -> (TempDir, PathBuf, PathBuf) {
    let dir = TempDir::new(&format!("bucket-{name}")).unwrap();
    let (blobs, database) = (dir.join("blobs"), dir.join("glave.db"));
    (dir, blobs, database)
}

async fn store(blobs: &Path, database: &Path) -> Store {
    let bucket = Bucket::folder(blobs).await.unwrap();
    let keys = KeySettings::new(Arc::new(InMemoryRoot::new().unwrap()));
    Store::open(database, keys, Arc::new(bucket)).await.unwrap()
}

/// Every file under `folder`, by its path from there.
fn files(folder: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut waiting = vec![folder.to_owned()];
    while let Some(next) = waiting.pop() {
        for entry in std::fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                waiting.push(path);
            } else {
                found.push(path.strip_prefix(folder).unwrap().to_owned());
            }
        }
    }
    found.sort();
    found
}

fn scope() -> Scope {
    Scope::glaves_own(
        OrgId::from_uuid(Uuid::from_u128(1)),
        ProjectId::from_uuid(Uuid::from_u128(2)),
    )
}

fn thread(n: u8) -> ThreadId {
    ThreadId::try_from(uuid::Builder::from_unix_timestamp_millis(1, &[n; 10]).into_uuid()).unwrap()
}

#[test]
fn a_folder_passes_the_objects_suite() {
    let (_dir, blobs, _) = places("objects");
    block_on!(async {
        let bucket = Bucket::folder(&blobs).await.unwrap();
        assert_eq!(conformance::objects(&bucket).await, Ok(()));
    });
    assert_eq!(files(&blobs), Vec::<PathBuf>::new(), "the suite tidies up");
}

#[test]
fn blobs_in_a_folder_pass_the_blob_store_suite() {
    let (_dir, blobs, database) = places("blobs");
    block_on!(async {
        let store = store(&blobs, &database).await;
        assert_eq!(conformance::blob_store(&store.blobs()).await, Ok(()));
    });
}

#[test]
fn the_folder_holds_only_sealed_bytes_under_random_names() {
    let (_dir, blobs, database) = places("sealed");
    block_on!(async {
        let store = store(&blobs, &database).await;
        let content = b"a screenshot, as it were".to_vec();
        store
            .blobs()
            .put(
                &scope(),
                thread(1),
                "image/png",
                1024,
                Box::new(Pieces::from(content.clone())),
            )
            .await
            .unwrap();
        let kept = files(&blobs);
        let [file] = kept.as_slice() else {
            panic!("one object: {kept:?}")
        };
        let parts: Vec<String> = file
            .iter()
            .map(|part| part.to_string_lossy().into_owned())
            .collect();
        let [org, project, object] = parts.as_slice() else {
            panic!("<organization>/<project>/<object>: {file:?}")
        };
        assert_eq!(
            (org.as_str(), project.as_str()),
            (
                scope().org().to_string().as_str(),
                scope().project().to_string().as_str()
            )
        );
        assert_ne!(object.as_str(), digest(&content).as_str());
        let bytes = std::fs::read(blobs.join(file)).unwrap();
        assert!(!bytes.windows(content.len()).any(|window| window == content));
    });
}

#[test]
fn a_refused_write_and_a_released_thread_leave_nothing_in_the_folder() {
    let (_dir, blobs, database) = places("tidy");
    block_on!(async {
        let store = store(&blobs, &database).await;
        let refused = store
            .blobs()
            .put(
                &scope(),
                thread(1),
                "image/png",
                10,
                Box::new(Pieces::from(vec![1; 11])),
            )
            .await;
        assert_eq!(refused, Err(BlobError::TooLarge(10)));
        assert_eq!(files(&blobs), Vec::<PathBuf>::new());
        store
            .blobs()
            .put(
                &scope(),
                thread(2),
                "text/plain",
                10,
                Box::new(Pieces::from(b"kept".to_vec())),
            )
            .await
            .unwrap();
        assert_eq!(files(&blobs).len(), 1);
        store.blobs().release(&scope(), thread(2)).await.unwrap();
        assert_eq!(files(&blobs), Vec::<PathBuf>::new());
    });
}
