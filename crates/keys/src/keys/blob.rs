//! Blobs sealed a chunk at a time (`specs/storage.md`, "Blobs"), so one of any size is written
//! and read without being held whole: each chunk of [`CHUNK`] bytes is sealed on its own, bound
//! to its object, its place and whether it is the last, so chunks cannot be dropped, reordered,
//! cut short or moved to another object; and the content read must have the hash it was asked
//! for.

use std::mem;

use glave_event::Sha256;
use glave_ports::Scope;
use sha2::Digest as _;

use crate::error::KeyError;
use crate::key_id::{Month, Period};
use crate::material::{KeyMaterial, random};
use crate::seal::{OVERHEAD, associated, open, seal};
use crate::storage::KeyStorage;

use super::Keys;

/// The plaintext bytes in each sealed chunk but the last, which holds 0 to `CHUNK`.
pub const CHUNK: usize = 64 * 1024;

/// The bytes of each sealed chunk but the last: [`CHUNK`] and what sealing adds.
pub const SEALED_CHUNK: usize = CHUNK + OVERHEAD;

/// How many chunks a blob of `size` bytes is sealed in: at least one, the last never empty
/// unless the blob is.
pub fn chunks_in(size: u64) -> u64 {
    size.div_ceil(CHUNK as u64).max(1)
}

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

/// Opens one blob's sealed chunks in order, and checks at the last that the content has the
/// hash it was asked for.
pub struct BlobOpener {
    key: KeyMaterial,
    key_id: String,
    scope: Scope,
    object: String,
    index: u32,
    hasher: sha2::Sha256,
    expected: Sha256,
    ended: bool,
}

impl BlobOpener {
    /// Opens the next chunk; `last` says whether the blob ends with it. Fails when it does not
    /// open in this place, and at the last when the content is not what was asked for.
    pub fn open(&mut self, sealed: &[u8], last: bool) -> Result<Vec<u8>, KeyError> {
        if self.ended {
            return Err(KeyError::Tampered(self.key_id.clone()));
        }
        let aad = chunk_aad(&self.scope, &self.object, &self.key_id, self.index, last);
        let opened = open(&self.key, &aad, sealed, &self.key_id)?;
        self.hasher.update(&opened);
        self.index = self
            .index
            .checked_add(1)
            .ok_or_else(|| KeyError::Tampered(self.key_id.clone()))?;
        if last {
            self.ended = true;
            let found = Sha256::from_digest(mem::take(&mut self.hasher).finalize().into());
            if found != self.expected {
                return Err(KeyError::Tampered(self.key_id.clone()));
            }
        }
        Ok(opened)
    }
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
            hasher: sha2::Sha256::new(),
            expected: sha256.clone(),
            ended: false,
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
