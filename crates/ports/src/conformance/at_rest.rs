//! What a store keeps at rest (`specs/storage.md`, "Keys"): every payload and blob sealed, a
//! payload bound to its row so one moved to another row, thread or project does not open, and
//! each blob kept under a keyed name with every thread that stored it. The store's own tests read
//! and write its rows straight from the database, through [`AtRest`].

use glave_event::{Seq, ThreadId};

use super::{Verdict, ensure, events, scope, thread};
use crate::blob::{BlobStore, Pieces, digest, read_all};
use crate::log::EventLog;
use crate::scope::Scope;
use crate::tools::BoxFuture;

/// One event's payload as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptPayload {
    pub bytes: Vec<u8>,
    /// The key that sealed it, when one did.
    pub key_id: Option<String>,
}

/// One blob as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptBlob {
    /// The name it is stored under.
    pub name: String,
    /// Its sealed chunks, one after another.
    pub bytes: Vec<u8>,
    /// Every thread recorded as storing it.
    pub threads: Vec<ThreadId>,
}

/// A store's rows, read and written around it, as an attacker with the database would.
pub trait AtRest: Send + Sync {
    /// The payload of event `seq` of `thread` in `scope`, as kept.
    fn payload(
        &self,
        scope: Scope,
        thread: ThreadId,
        seq: Seq,
    ) -> BoxFuture<'_, Result<KeptPayload, String>>;

    /// Writes `kept` over the payload of event `seq` of `thread` in `scope`.
    fn overwrite(
        &self,
        scope: Scope,
        thread: ThreadId,
        seq: Seq,
        kept: KeptPayload,
    ) -> BoxFuture<'_, Result<(), String>>;

    /// Every blob kept in `scope`.
    fn blobs(&self, scope: Scope) -> BoxFuture<'_, Result<Vec<KeptBlob>, String>>;
}

/// The suite, on a store's log and blobs and its rows read straight from its database. The
/// store must be empty.
pub async fn at_rest<L: EventLog, B: BlobStore>(log: &L, blobs: &B, rows: &dyn AtRest) -> Verdict {
    sealed_and_bound(log, rows).await?;
    blobs_kept(blobs, rows).await
}

/// Payloads are sealed under a key, read back as written, and open only in their own row.
async fn sealed_and_bound<L: EventLog>(log: &L, rows: &dyn AtRest) -> Verdict {
    let (here, there) = (scope(41), scope(42));
    let (a, b) = (thread(41)?, thread(42)?);
    let written = events(a, &[1, 2])?;
    log.append(&here, &written)
        .await
        .map_err(|e| e.to_string())?;
    log.append(&here, &events(b, &[1])?)
        .await
        .map_err(|e| e.to_string())?;
    log.append(&there, &events(a, &[1])?)
        .await
        .map_err(|e| e.to_string())?;
    let first = rows.payload(here, a, Seq::FIRST).await?;
    ensure(
        first.key_id.is_some(),
        "a payload must be kept with the key that sealed it",
    )?;
    ensure(
        !String::from_utf8_lossy(&first.bytes).contains("completed"),
        "a payload must not be kept in the clear",
    )?;
    let read = |scope: Scope, thread: ThreadId| async move {
        log.read(&scope, thread, None, usize::MAX).await
    };
    let back = read(here, a).await.map_err(|e| e.to_string())?;
    ensure(
        back.iter().map(|e| e.to_json().ok()).collect::<Vec<_>>()
            == written.iter().map(|e| e.to_json().ok()).collect::<Vec<_>>(),
        "sealed payloads must read back exactly as written",
    )?;
    let second = Seq::new(2).map_err(|e| e.to_string())?;
    for (scope, target, seq, place) in [
        (here, a, second, "another row"),
        (here, b, Seq::FIRST, "another thread"),
        (there, a, Seq::FIRST, "another project"),
    ] {
        let original = rows.payload(scope, target, seq).await?;
        rows.overwrite(scope, target, seq, first.clone()).await?;
        let moved = read(scope, target).await;
        rows.overwrite(scope, target, seq, original).await?;
        ensure(
            moved.is_err(),
            &format!("a payload moved to {place} must not open"),
        )?;
        ensure(
            read(scope, target).await.is_ok(),
            "a payload put back must open again",
        )?;
    }
    Ok(())
}

/// A blob is kept once, sealed, under a name that is not its hash, with each thread that
/// stored it.
async fn blobs_kept<B: BlobStore>(blobs: &B, rows: &dyn AtRest) -> Verdict {
    let here = scope(43);
    let (a, b) = (thread(43)?, thread(44)?);
    let bytes = b"shared at rest".to_vec();
    for by in [a, b] {
        blobs
            .put(
                &here,
                Some(by),
                "text/plain",
                1024,
                Box::new(Pieces::from(bytes.clone())),
            )
            .await
            .map_err(|e| e.to_string())?;
    }
    let kept = rows.blobs(here).await?;
    let [blob] = kept.as_slice() else {
        return Err(format!(
            "identical content must be kept once, not {}",
            kept.len()
        ));
    };
    ensure(
        blob.name != digest(&bytes).as_str(),
        "a blob must be kept under a keyed name, not its hash",
    )?;
    ensure(
        !String::from_utf8_lossy(&blob.bytes).contains("shared at rest"),
        "a blob must not be kept in the clear",
    )?;
    let mut threads = blob.threads.clone();
    threads.sort();
    let mut both = vec![a, b];
    both.sort();
    ensure(
        threads == both,
        "every thread that stored a blob must be recorded with it",
    )?;
    let mut got = blobs
        .get(&here, &digest(&bytes))
        .await
        .map_err(|e| e.to_string())?
        .ok_or("a sealed blob must be found")?;
    let read_back = read_all(got.bytes.as_mut(), 1024)
        .await
        .map_err(|e| e.to_string())?;
    ensure(read_back == bytes, "a sealed blob must read back exactly")
}
