//! An S3-compatible bucket: the objects suite and the blob store suite under the local
//! database, against the bucket `GLAVE_TEST_S3_URL` names (`s3://name`) at the endpoint
//! `GLAVE_TEST_S3_ENDPOINT` names, unsigned, as a SeaweedFS with no keys takes it. CI runs them
//! against SeaweedFS; without the variables they are skipped.

use std::sync::Arc;

use glave_bucket::Bucket;
use glave_keys::{InMemoryRoot, KeySettings};
use glave_ports::testing::TempDir;
use glave_ports::{block_on, conformance};
use glave_store_sqlite::Store;

/// The bucket the tests may use, or `None` when none was given.
fn bucket() -> Option<Bucket> {
    let (Ok(url), Ok(endpoint)) = (
        std::env::var("GLAVE_TEST_S3_URL"),
        std::env::var("GLAVE_TEST_S3_ENDPOINT"),
    ) else {
        eprintln!("skipped: set GLAVE_TEST_S3_URL and GLAVE_TEST_S3_ENDPOINT to run the S3 tests");
        return None;
    };
    let settings = [
        ("AWS_ENDPOINT_URL", endpoint.as_str()),
        ("AWS_REGION", "us-east-1"),
        ("AWS_ALLOW_HTTP", "true"),
        ("AWS_SKIP_SIGNATURE", "true"),
        ("GLAVE_NOT_AWS", "ignored"),
    ];
    let settings = settings.map(|(name, value)| (name.to_owned(), value.to_owned()));
    Some(Bucket::s3(&url, settings).unwrap())
}

#[test]
fn a_bucket_passes_the_objects_suite() {
    let Some(bucket) = bucket() else {
        return;
    };
    assert_eq!(block_on!(conformance::objects(&bucket)), Ok(()));
}

#[test]
fn blobs_in_a_bucket_pass_the_blob_store_suite() {
    let Some(bucket) = bucket() else {
        return;
    };
    let dir = TempDir::new("bucket-s3").unwrap();
    block_on!(async {
        let keys = KeySettings::new(Arc::new(InMemoryRoot::new().unwrap()));
        let store = Store::open(dir.join("glave.db"), keys, Arc::new(bucket))
            .await
            .unwrap();
        assert_eq!(conformance::blob_store(&store.blobs()).await, Ok(()));
    });
}
