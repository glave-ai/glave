//! Blobs sealed a chunk at a time (`specs/storage.md`, "Blobs"), so one of any size is written
//! and read, in pieces of any size, without being held whole: each chunk of [`CHUNK`] bytes is
//! sealed on its own, bound to its object, its place and whether it is the last, so chunks cannot be dropped, reordered,
//! cut short or moved to another object; and the content read must have the hash it was asked
//! for.

use std::mem;

use glave_event::Sha256;
use glave_ports::{BlobError, BoxFuture, Chunks, ObjectWriter, Scope};
use sha2::Digest as _;

use crate::error::KeyError;
use crate::key_id::{Month, Period};
use crate::material::{KeyMaterial, random};
use crate::seal::{OVERHEAD, associated, open, seal};
use crate::storage::KeyStorage;

use super::Keys;

/// The plaintext bytes in each sealed chunk but the last, which holds 0 to `CHUNK`.
pub(crate) const CHUNK: usize = 64 * 1024;

/// The bytes of each sealed chunk but the last: [`CHUNK`] and what sealing adds.
pub(crate) const SEALED_CHUNK: usize = CHUNK + OVERHEAD;

/// Seals one blob as its bytes arrive, under this month's data key, for the object `object`
/// names.
pub struct BlobSealer {
    key: KeyMaterial,
    key_id: String,
    scope: Scope,
    object: String,
    index: u32,
    pending: Vec<u8>,
    hasher: sha2::Sha256,
    size: u64,
}

/// What [`BlobSealer::finish`] leaves: the last chunk, and the content's hash and size.
#[derive(Debug)]
pub struct Finished {
    pub last: Vec<u8>,
    pub sha256: Sha256,
    pub size: u64,
}

impl BlobSealer {
    /// The data key sealing it, as the record keeps it.
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    /// The object its sealed chunks are kept as: random, so it says nothing of the content.
    pub fn object(&self) -> &str {
        &self.object
    }

    /// The bytes taken in so far.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Takes in `bytes`, and returns the chunks it can seal now: every full chunk that more
    /// bytes follow.
    pub fn write(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, KeyError> {
        self.hasher.update(bytes);
        self.size = self.size.saturating_add(bytes.len() as u64);
        let mut pending = mem::take(&mut self.pending);
        pending.extend_from_slice(bytes);
        let mut sealed = Vec::new();
        let mut start = 0;
        while pending.len().saturating_sub(start) > CHUNK {
            let end = start + CHUNK;
            let chunk = pending.get(start..end).unwrap_or_default();
            sealed.push(self.seal(chunk, false)?);
            start = end;
        }
        pending.drain(..start);
        self.pending = pending;
        Ok(sealed)
    }

    /// Seals what is left as the last chunk.
    pub fn finish(mut self) -> Result<Finished, KeyError> {
        let pending = mem::take(&mut self.pending);
        let last = self.seal(&pending, true)?;
        Ok(Finished {
            last,
            sha256: Sha256::from_digest(self.hasher.finalize().into()),
            size: self.size,
        })
    }

    fn seal(&mut self, chunk: &[u8], last: bool) -> Result<Vec<u8>, KeyError> {
        let aad = chunk_aad(&self.scope, &self.object, &self.key_id, self.index, last);
        let sealed = seal(&self.key, &aad, chunk)?;
        self.index = self
            .index
            .checked_add(1)
            .ok_or_else(|| KeyError::Corrupt("a blob is too large to seal".into()))?;
        Ok(sealed)
    }
}

/// Opens one blob's sealed bytes as they arrive, in pieces of any size, and checks at the end
/// that the content has the hash it was asked for.
pub struct BlobOpener {
    key: KeyMaterial,
    key_id: String,
    scope: Scope,
    object: String,
    index: u32,
    pending: Vec<u8>,
    hasher: sha2::Sha256,
    expected: Sha256,
}

impl BlobOpener {
    /// Takes in sealed `bytes`, and returns what it can open now: every whole chunk that more
    /// bytes follow.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<u8>, KeyError> {
        let mut pending = mem::take(&mut self.pending);
        pending.extend_from_slice(bytes);
        let mut opened = Vec::new();
        let mut start = 0;
        while pending.len().saturating_sub(start) > SEALED_CHUNK {
            let end = start + SEALED_CHUNK;
            let chunk = pending.get(start..end).unwrap_or_default();
            opened.extend(self.open(chunk, false)?);
            start = end;
        }
        pending.drain(..start);
        self.pending = pending;
        Ok(opened)
    }

    /// Opens what is left as the last chunk. Fails when it does not open there, as when the
    /// bytes were cut short, and when the content is not what was asked for.
    pub fn end(mut self) -> Result<Vec<u8>, KeyError> {
        let pending = mem::take(&mut self.pending);
        let opened = self.open(&pending, true)?;
        let found = Sha256::from_digest(mem::take(&mut self.hasher).finalize().into());
        if found == self.expected {
            Ok(opened)
        } else {
            Err(KeyError::Tampered(self.key_id.clone()))
        }
    }

    fn open(&mut self, sealed: &[u8], last: bool) -> Result<Vec<u8>, KeyError> {
        let aad = chunk_aad(&self.scope, &self.object, &self.key_id, self.index, last);
        let opened = open(&self.key, &aad, sealed, &self.key_id)?;
        self.hasher.update(&opened);
        self.index = self
            .index
            .checked_add(1)
            .ok_or_else(|| KeyError::Tampered(self.key_id.clone()))?;
        Ok(opened)
    }
}

/// Seals what `bytes` yields into `writer`, refusing more than `limit` bytes. The writer is
/// left to finish or abandon.
pub async fn seal_into(
    mut sealer: BlobSealer,
    limit: u64,
    bytes: &mut dyn Chunks,
    writer: &mut dyn ObjectWriter,
) -> Result<Finished, BlobError> {
    while let Some(piece) = bytes.next_chunk().await? {
        if sealer.size().saturating_add(piece.len() as u64) > limit {
            return Err(BlobError::TooLarge(limit));
        }
        for chunk in sealer.write(&piece).map_err(|e| sealing(&e))? {
            writer.write(chunk).await?;
        }
    }
    let finished = sealer.finish().map_err(|e| sealing(&e))?;
    writer.write(finished.last.clone()).await?;
    Ok(finished)
}

/// A blob's content: its object's sealed bytes, opened as they are read.
pub struct Opened {
    sealed: Box<dyn Chunks>,
    opener: Option<BlobOpener>,
}

impl Opened {
    pub fn new(sealed: Box<dyn Chunks>, opener: BlobOpener) -> Self {
        Self {
            sealed,
            opener: Some(opener),
        }
    }

    async fn next_now(&mut self) -> Result<Option<Vec<u8>>, BlobError> {
        loop {
            let Some(reading) = self.opener.as_mut() else {
                return Ok(None);
            };
            let Some(piece) = self.sealed.next_chunk().await? else {
                let last = self.opener.take().map(BlobOpener::end).transpose();
                return last.map_err(|e| sealing(&e));
            };
            let content = reading.feed(&piece).map_err(|e| sealing(&e))?;
            if !content.is_empty() {
                return Ok(Some(content));
            }
        }
    }
}

impl Chunks for Opened {
    fn next_chunk(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>, BlobError>> {
        Box::pin(self.next_now())
    }
}

/// Why a blob could not be sealed or opened, as the blob store says it.
fn sealing(error: &KeyError) -> BlobError {
    BlobError::Storage(format!("a blob could not be sealed or opened: {error}"))
}

impl<S: KeyStorage> Keys<S> {
    /// A sealer for a new blob in `scope`, under the project's data key for this month.
    pub async fn blob_sealer(&self, scope: &Scope) -> Result<BlobSealer, KeyError> {
        let month = Month::of(self.clock.now());
        let (id, key) = self.sealing_key(scope, Period::Month(month)).await?;
        let mut object = [0u8; 16];
        random(&mut object)?;
        Ok(BlobSealer {
            key,
            key_id: id.to_string(),
            scope: *scope,
            object: uuid::Uuid::from_bytes(object).simple().to_string(),
            index: 0,
            pending: Vec::new(),
            hasher: sha2::Sha256::new(),
            size: 0,
        })
    }

    /// An opener for the blob whose content hashes to `sha256`, sealed under `key_id` as
    /// `object`.
    pub async fn blob_opener(
        &self,
        scope: &Scope,
        key_id: &str,
        object: &str,
        sha256: &Sha256,
    ) -> Result<BlobOpener, KeyError> {
        let key = self.data_key(scope, key_id.parse()?).await?;
        Ok(BlobOpener {
            key,
            key_id: key_id.to_owned(),
            scope: *scope,
            object: object.to_owned(),
            index: 0,
            pending: Vec::new(),
            hasher: sha2::Sha256::new(),
            expected: sha256.clone(),
        })
    }
}

/// What binds a sealed chunk to its scope, its object, its key, its place and whether it is
/// the last.
fn chunk_aad(scope: &Scope, object: &str, key_id: &str, index: u32, last: bool) -> Vec<u8> {
    associated(&[
        b"glave/blob/v2",
        scope.org().to_string().as_bytes(),
        scope.project().to_string().as_bytes(),
        object.as_bytes(),
        key_id.as_bytes(),
        &index.to_be_bytes(),
        &[u8::from(last)],
    ])
}

#[cfg(test)]
mod tests;
