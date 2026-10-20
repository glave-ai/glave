//! A thread's blobs: deleting a thread deletes the blobs only it used, and keeps those another
//! thread still uses.

use glave_harness::{Change, NoOneToAsk};
use glave_ports::{BlobStore as _, Caller, Pieces};

use crate::support::*;

#[test]
fn deleting_a_thread_deletes_the_blobs_only_it_used() {
    let dir = folder("blobs-deleted");
    block_on!(async {
        let harness = harness(&dir, Vec::new(), NoOneToAsk).await;
        let mut a = harness.start_thread(scope()).await.unwrap();
        let mut b = harness.start_thread(scope()).await.unwrap();
        let (blobs, here) = (harness.blobs(), scope());
        let put = |thread, bytes: &[u8]| {
            blobs.put(
                &here,
                thread,
                "text/plain",
                64,
                Box::new(Pieces::from(bytes.to_vec())),
            )
        };
        let shared = put(a.id, b"shared").await.unwrap();
        put(b.id, b"shared").await.unwrap();
        let only_a = put(a.id, b"only a").await.unwrap();
        let found = |sha| async move { blobs.get(&here, &sha).await.unwrap().is_some() };

        harness
            .change_standing(&mut a, &Caller::default(), Change::Archive)
            .await
            .unwrap();
        assert!(found(only_a.sha256.clone()).await, "archiving keeps them");
        harness
            .change_standing(&mut a, &Caller::default(), Change::Delete)
            .await
            .unwrap();
        assert!(
            !found(only_a.sha256).await,
            "only the deleted thread used it"
        );
        assert!(found(shared.sha256.clone()).await, "another thread uses it");
        harness
            .change_standing(&mut b, &Caller::default(), Change::Delete)
            .await
            .unwrap();
        assert!(!found(shared.sha256).await, "its last thread was deleted");
    });
}
