//! What every blob store must do (`specs/storage.md`, "Blobs"): name content by its SHA-256 and
//! keep one copy of it per scope, read it back exactly however it was cut into pieces, refuse
//! more than the limit and keep nothing of it, keep each scope's blobs apart, and keep a blob
//! while a thread refers to it: releasing a thread deletes only the blobs no other thread uses.

use glave_event::{BlobRef, Sha256, ThreadId};

use super::{Verdict, ensure, scope, thread};
use crate::blob::{BlobError, BlobStore, Pieces, digest, read_all};
use crate::scope::Scope;

/// A limit no blob here reaches.
const ROOMY: u64 = 64 * 1024 * 1024;

/// The suite. The store must be empty.
pub async fn blob_store<B: BlobStore>(blobs: &B) -> Verdict {
    named_kept_once_and_isolated(blobs).await?;
    large_and_empty(blobs).await?;
    over_the_limit(blobs).await?;
    released(blobs).await
}

async fn put<B: BlobStore>(
    blobs: &B,
    scope: &Scope,
    thread: ThreadId,
    pieces: Pieces,
) -> Result<BlobRef, String> {
    blobs
        .put(scope, thread, "application/json", ROOMY, Box::new(pieces))
        .await
        .map_err(|e| e.to_string())
}

/// The content stored under `sha256`, read whole, or `None`.
async fn read<B: BlobStore>(
    blobs: &B,
    scope: &Scope,
    sha256: &Sha256,
) -> Result<Option<(BlobRef, Vec<u8>)>, String> {
    let Some(mut blob) = blobs.get(scope, sha256).await.map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let bytes = read_all(blob.bytes.as_mut(), ROOMY)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Some((blob.reference, bytes)))
}

async fn named_kept_once_and_isolated<B: BlobStore>(blobs: &B) -> Verdict {
    let (here, elsewhere) = (scope(1), scope(2));
    let bytes = b"turn config".to_vec();
    let t = thread(1)?;
    let reference = put(blobs, &here, t, Pieces::from(bytes.clone())).await?;
    ensure(
        reference.sha256 == digest(&bytes),
        "a blob must be named by its SHA-256",
    )?;
    ensure(
        reference.bytes == 11,
        "a blob reference must record its size",
    )?;
    ensure(
        reference.media_type == "application/json",
        "a blob reference must keep its media type",
    )?;
    let again = put(blobs, &here, t, Pieces::from(bytes.clone())).await?;
    ensure(
        again == reference,
        "identical content must get the identical reference",
    )?;
    ensure(
        read(blobs, &here, &reference.sha256).await? == Some((reference.clone(), bytes)),
        "a blob must read back exactly, with its reference",
    )?;
    ensure(
        read(blobs, &elsewhere, &reference.sha256).await?.is_none(),
        "another scope must not see this blob",
    )?;
    ensure(
        read(blobs, &here, &digest(b"never stored"))
            .await?
            .is_none(),
        "unknown content must read as missing",
    )
}

/// A blob of several megabytes, given in uneven pieces, reads back whole; so does an empty one.
async fn large_and_empty<B: BlobStore>(blobs: &B) -> Verdict {
    let here = scope(3);
    let t = thread(3)?;
    let large: Vec<u8> = (0..3_u32 * 1024 * 1024 + 7)
        .map(|n| n.to_le_bytes().iter().fold(0, |a: u8, b| a ^ b))
        .collect();
    let pieces: Pieces = large.chunks(100_003).map(<[u8]>::to_vec).collect();
    let reference = put(blobs, &here, t, pieces).await?;
    ensure(
        reference.sha256 == digest(&large) && reference.bytes == large.len() as u64,
        "a large blob given in pieces must be named and sized as one",
    )?;
    let read_back = read(blobs, &here, &reference.sha256).await?;
    ensure(
        read_back.is_some_and(|(_, bytes)| bytes == large),
        "a large blob must read back exactly",
    )?;
    let empty = put(blobs, &here, t, Pieces::default()).await?;
    ensure(
        empty.sha256 == digest(b"") && empty.bytes == 0,
        "an empty blob is the hash of nothing",
    )?;
    ensure(
        read(blobs, &here, &empty.sha256).await? == Some((empty, Vec::new())),
        "an empty blob must read back empty",
    )
}

/// More than the limit is refused, and nothing of it is kept.
async fn over_the_limit<B: BlobStore>(blobs: &B) -> Verdict {
    let here = scope(4);
    let bytes = vec![9u8; 200_000];
    let refused = blobs
        .put(
            &here,
            thread(4)?,
            "image/png",
            199_999,
            Box::new(bytes.chunks(70_000).map(<[u8]>::to_vec).collect::<Pieces>()),
        )
        .await;
    ensure(
        refused == Err(BlobError::TooLarge(199_999)),
        "content over the limit must be refused as too large",
    )?;
    ensure(
        read(blobs, &here, &digest(&bytes)).await?.is_none(),
        "content refused must not be kept",
    )?;
    let at_the_limit = blobs
        .put(
            &here,
            thread(4)?,
            "image/png",
            200_000,
            Box::new(Pieces::from(bytes)),
        )
        .await;
    ensure(
        at_the_limit.is_ok(),
        "content exactly at the limit must be kept",
    )
}

/// Releasing a thread deletes the blobs only it used; one another thread uses stays until that
/// one is released too; other scopes' are untouched.
async fn released<B: BlobStore>(blobs: &B) -> Verdict {
    let (here, elsewhere) = (scope(5), scope(6));
    let (a, b) = (thread(5)?, thread(6)?);
    let shared = put(blobs, &here, a, Pieces::from(b"shared".to_vec())).await?;
    put(blobs, &here, b, Pieces::from(b"shared".to_vec())).await?;
    let only_a = put(blobs, &here, a, Pieces::from(b"only a".to_vec())).await?;
    let only_b = put(blobs, &here, b, Pieces::from(b"only b".to_vec())).await?;
    let a_elsewhere = put(blobs, &elsewhere, a, Pieces::from(b"only a".to_vec())).await?;
    let release = |thread| async move {
        blobs
            .release(&here, thread)
            .await
            .map_err(|e| e.to_string())
    };
    release(a).await?;
    ensure(
        read(blobs, &here, &only_a.sha256).await?.is_none(),
        "a blob only a released thread used must be deleted",
    )?;
    ensure(
        read(blobs, &here, &shared.sha256).await?.is_some(),
        "a blob another thread still uses must stay",
    )?;
    ensure(
        read(blobs, &here, &only_b.sha256).await?.is_some(),
        "another thread's own blobs must stay",
    )?;
    ensure(
        read(blobs, &elsewhere, &a_elsewhere.sha256)
            .await?
            .is_some(),
        "releasing a thread in one scope must not touch another scope's blobs",
    )?;
    release(a).await?;
    release(b).await?;
    ensure(
        read(blobs, &here, &shared.sha256).await?.is_none(),
        "a blob must be deleted once its last thread is released",
    )?;
    let again = put(blobs, &here, b, Pieces::from(b"shared".to_vec())).await?;
    ensure(
        again == shared && read(blobs, &here, &shared.sha256).await?.is_some(),
        "content deleted can be stored again",
    )
}
