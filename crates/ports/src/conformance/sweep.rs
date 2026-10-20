//! Eviction by reference, not age (`specs/storage.md`, "Blobs"): a blob lives while a thread
//! refers to it. The sweep deletes blobs whose last reference went, uploads no thread referred
//! to within a day, and bytes no record names once they are a day old; it keeps a blob two
//! threads share when one is released, and the store orders a reference and a sweep, not the
//! clock.

use super::blobs::{put, read};
use super::{Verdict, ensure, scope, thread};
use crate::blob::{BlobStore, Pieces, Swept, UNREFERENCED_FOR_MS};
use crate::objects::Objects;
use crate::scope::Scope;
use crate::testing::ManualClock;

/// The sweep's cases, on a store whose clock is `clock`, and with `objects` its bytes' place.
pub(super) async fn sweeping<B: BlobStore>(
    blobs: &B,
    clock: &ManualClock,
    objects: Option<&dyn Objects>,
) -> Verdict {
    released(blobs).await?;
    uploads(blobs, clock).await?;
    referred_or_swept_first(blobs, clock).await?;
    if let Some(objects) = objects {
        orphans(blobs, clock, objects).await?;
    }
    Ok(())
}

async fn sweep<B: BlobStore>(blobs: &B, scope: Scope) -> Result<Swept, String> {
    blobs.sweep(scope.org()).await.map_err(|e| e.to_string())
}

async fn upload<B: BlobStore>(blobs: &B, scope: &Scope, bytes: &[u8]) -> Result<(), String> {
    blobs
        .put(
            scope,
            None,
            "image/png",
            1024,
            Box::new(Pieces::from(bytes.to_vec())),
        )
        .await
        .map(drop)
        .map_err(|e| e.to_string())
}

/// Releasing a thread leaves its blobs to the sweep, which deletes those only it used; one
/// another thread uses stays until that one is released too; other organizations' are
/// untouched.
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
        read(blobs, &here, &only_a.sha256).await?.is_some(),
        "a released thread's blobs must stay until the sweep",
    )?;
    let swept = sweep(blobs, here).await?;
    ensure(
        swept.blobs == 1,
        "the sweep must delete exactly the blob whose last reference went",
    )?;
    ensure(
        read(blobs, &here, &only_a.sha256).await?.is_none(),
        "a blob only a released thread used must be swept",
    )?;
    ensure(
        read(blobs, &here, &shared.sha256).await?.is_some(),
        "a blob two threads share must survive releasing one of them",
    )?;
    ensure(
        read(blobs, &here, &only_b.sha256).await?.is_some(),
        "another thread's own blobs must stay",
    )?;
    ensure(
        read(blobs, &elsewhere, &a_elsewhere.sha256)
            .await?
            .is_some(),
        "releasing and sweeping one organization must not touch another's blobs",
    )?;
    release(a).await?;
    release(b).await?;
    sweep(blobs, here).await?;
    ensure(
        read(blobs, &here, &shared.sha256).await?.is_none(),
        "a blob must be swept once its last thread is released",
    )?;
    let again = put(blobs, &here, b, Pieces::from(b"shared".to_vec())).await?;
    ensure(
        again == shared && read(blobs, &here, &shared.sha256).await?.is_some(),
        "content swept can be stored again",
    )
}

/// An upload no thread refers to is kept a day, then swept; uploading it again keeps it a day
/// from then; one a thread came to refer to stays.
async fn uploads<B: BlobStore>(blobs: &B, clock: &ManualClock) -> Verdict {
    let here = scope(7);
    let (draft, sent) = (b"a draft's screenshot", b"a sent screenshot");
    upload(blobs, &here, draft).await?;
    upload(blobs, &here, sent).await?;
    let referred = blobs
        .refer(&here, &crate::blob::digest(sent), thread(7)?)
        .await
        .map_err(|e| e.to_string())?;
    ensure(referred, "an upload must take a reference while it is kept")?;
    clock.advance_millis(UNREFERENCED_FOR_MS - 1);
    upload(blobs, &here, draft).await?;
    clock.advance_millis(2);
    sweep(blobs, here).await?;
    ensure(
        read(blobs, &here, &crate::blob::digest(draft))
            .await?
            .is_some(),
        "an upload made again must be kept a day from then",
    )?;
    clock.advance_millis(UNREFERENCED_FOR_MS);
    let swept = sweep(blobs, here).await?;
    ensure(
        swept.blobs == 1
            && read(blobs, &here, &crate::blob::digest(draft))
                .await?
                .is_none(),
        "an upload no thread referred to within a day must be swept",
    )?;
    ensure(
        read(blobs, &here, &crate::blob::digest(sent))
            .await?
            .is_some(),
        "an upload a thread refers to must stay",
    )
}

/// A reference and a sweep are ordered by the store: one taken first keeps the blob; once the
/// sweep deleted it, a reference finds nothing to refer to.
async fn referred_or_swept_first<B: BlobStore>(blobs: &B, clock: &ManualClock) -> Verdict {
    let here = scope(8);
    let (kept, late) = (b"referred in time", b"referred too late");
    upload(blobs, &here, kept).await?;
    upload(blobs, &here, late).await?;
    clock.advance_millis(UNREFERENCED_FOR_MS + 1);
    let refer = |bytes: &'static [u8]| async move {
        blobs
            .refer(&here, &crate::blob::digest(bytes), thread(8)?)
            .await
            .map_err(|e| e.to_string())
    };
    ensure(
        refer(kept).await?,
        "a reference taken before the sweep must find the blob",
    )?;
    sweep(blobs, here).await?;
    ensure(
        read(blobs, &here, &crate::blob::digest(kept))
            .await?
            .is_some(),
        "a blob referred to before the sweep must stay",
    )?;
    ensure(
        !refer(late).await?,
        "a reference after the sweep deleted the blob must find nothing",
    )?;
    ensure(
        read(blobs, &here, &crate::blob::digest(late))
            .await?
            .is_none(),
        "a reference that found nothing must not bring the blob back",
    )
}

/// Bytes with no record (a crash between writing them and their record) are kept while they
/// may be a write in progress, then swept; a blob's own bytes stay.
async fn orphans<B: BlobStore>(blobs: &B, clock: &ManualClock, objects: &dyn Objects) -> Verdict {
    let here = scope(9);
    let live = put(blobs, &here, thread(9)?, Pieces::from(b"live".to_vec())).await?;
    let mut writer = objects
        .create(here, "0dd0b1ec75")
        .await
        .map_err(|e| e.to_string())?;
    writer
        .write(b"sealed bytes a crash left".to_vec())
        .await
        .map_err(|e| e.to_string())?;
    writer.finish().await.map_err(|e| e.to_string())?;
    let written = objects
        .list(here.org())
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|listed| listed.object == "0dd0b1ec75")
        .ok_or("an object written must be listed")?
        .written_ms;
    // A bucket kept between runs may hold older orphans of this organization too, which the
    // sweep rightly takes: what is checked is this one.
    let kept = || async {
        Ok::<bool, String>(
            objects
                .list(here.org())
                .await
                .map_err(|e| e.to_string())?
                .iter()
                .any(|listed| listed.object == "0dd0b1ec75"),
        )
    };
    clock.set_millis(written + UNREFERENCED_FOR_MS - 1);
    sweep(blobs, here).await?;
    ensure(
        kept().await?,
        "bytes with no record must be kept for a day: they may be a write in progress",
    )?;
    clock.set_millis(written + UNREFERENCED_FOR_MS + 1);
    let swept = sweep(blobs, here).await?;
    ensure(
        swept.orphans >= 1 && !kept().await?,
        "bytes with no record a day old must be swept, and gone from where they were kept",
    )?;
    ensure(
        read(blobs, &here, &live.sha256).await?.is_some(),
        "a blob's own bytes must never be swept as orphans",
    )
}
