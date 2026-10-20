use std::sync::Arc;

use glave_ports::testing::ManualClock;
use glave_ports::{OrgId, ProjectId, block_on, digest};
use uuid::Uuid;

use super::*;
use crate::key_id::KeyId;
use crate::keys::KeySettings;
use crate::root::InMemoryRoot;
use crate::storage::InMemoryKeyStorage;

fn keys() -> Keys<InMemoryKeyStorage> {
    let mut settings = KeySettings::new(Arc::new(InMemoryRoot::new().unwrap()));
    let at: jiff::Timestamp = "2026-10-10T12:00:00Z".parse().unwrap();
    settings.clock = Arc::new(ManualClock::at_millis(at.as_millisecond()));
    Keys::new(settings, InMemoryKeyStorage::new())
}

fn scope(project: u128) -> Scope {
    Scope::glaves_own(
        OrgId::from_uuid(Uuid::from_u128(1)),
        ProjectId::from_uuid(Uuid::from_u128(project)),
    )
}

/// `content` sealed in pieces of `piece` bytes: its chunks, last included, and what finished.
fn sealed(
    keys: &Keys<InMemoryKeyStorage>,
    content: &[u8],
    piece: usize,
) -> (BlobSealer, Vec<Vec<u8>>) {
    let mut sealer = block_on!(keys.blob_sealer(&scope(2))).unwrap();
    let mut chunks = Vec::new();
    for part in content.chunks(piece.max(1)) {
        chunks.extend(sealer.write(part).unwrap());
    }
    (sealer, chunks)
}

fn opened(
    keys: &Keys<InMemoryKeyStorage>,
    object: &str,
    key_id: &str,
    sha: &Sha256,
    chunks: &[Vec<u8>],
) -> Result<Vec<u8>, KeyError> {
    let mut opener = block_on!(keys.blob_opener(&scope(2), key_id, object, sha))?;
    let mut out = Vec::new();
    for (n, chunk) in chunks.iter().enumerate() {
        out.extend(opener.open(chunk, n + 1 == chunks.len())?);
    }
    Ok(out)
}

#[test]
fn a_blob_sealed_in_any_pieces_opens_whole_in_chunks_of_a_fixed_size() {
    let keys = keys();
    let content: Vec<u8> = (0..3 * CHUNK + 17)
        .map(|n| u8::try_from(n % 251).unwrap())
        .collect();
    for piece in [1000, CHUNK, CHUNK + 1, content.len()] {
        let (sealer, mut chunks) = sealed(&keys, &content, piece);
        let (object, key_id) = (sealer.object().to_owned(), sealer.key_id().to_owned());
        let finished = sealer.finish().unwrap();
        chunks.push(finished.last);
        assert_eq!(chunks.len() as u64, chunks_in(content.len() as u64));
        assert!(
            chunks
                .iter()
                .rev()
                .skip(1)
                .all(|chunk| chunk.len() == SEALED_CHUNK)
        );
        assert_eq!(finished.sha256, digest(&content));
        assert_eq!(finished.size, content.len() as u64);
        assert_eq!(
            opened(&keys, &object, &key_id, &finished.sha256, &chunks).unwrap(),
            content
        );
    }
}

#[test]
fn an_empty_blob_is_one_empty_chunk_and_a_full_last_chunk_stays_last() {
    let keys = keys();
    assert_eq!(chunks_in(0), 1);
    assert_eq!(chunks_in(CHUNK as u64), 1);
    assert_eq!(chunks_in(CHUNK as u64 + 1), 2);
    let (sealer, chunks) = sealed(&keys, &vec![0; CHUNK], CHUNK);
    assert_eq!(
        chunks.len(),
        0,
        "a full chunk waits to learn if it is the last"
    );
    assert_eq!(sealer.finish().unwrap().last.len(), SEALED_CHUNK);
    let (sealer, chunks) = sealed(&keys, b"", 1);
    let (object, key_id) = (sealer.object().to_owned(), sealer.key_id().to_owned());
    let finished = sealer.finish().unwrap();
    assert_eq!(chunks.len(), 0);
    assert_eq!(
        opened(&keys, &object, &key_id, &digest(b""), &[finished.last]).unwrap(),
        b""
    );
}

#[test]
fn chunks_cut_short_reordered_moved_or_of_other_content_do_not_open() {
    let keys = keys();
    let content = vec![7u8; 2 * CHUNK + 5];
    let (sealer, mut chunks) = sealed(&keys, &content, CHUNK);
    let (object, key_id) = (sealer.object().to_owned(), sealer.key_id().to_owned());
    let finished = sealer.finish().unwrap();
    chunks.push(finished.last);
    let sha = finished.sha256;
    let tampered = Err(KeyError::Tampered(key_id.clone()));
    let short = chunks.get(..2).unwrap();
    assert_eq!(opened(&keys, &object, &key_id, &sha, short), tampered);
    let mut swapped = chunks.clone();
    swapped.swap(0, 1);
    assert_eq!(opened(&keys, &object, &key_id, &sha, &swapped), tampered);
    let (other, _) = sealed(&keys, b"x", 1);
    assert_eq!(
        opened(&keys, other.object(), &key_id, &sha, &chunks),
        tampered
    );
    assert_eq!(
        opened(&keys, &object, &key_id, &digest(b"other"), &chunks),
        tampered,
        "content must have the hash it was asked for"
    );
}

#[test]
fn blobs_are_sealed_under_the_clocks_month_as_random_objects() {
    let keys = keys();
    let (first, _) = sealed(&keys, b"x", 1);
    let (second, _) = sealed(&keys, b"x", 1);
    let id: KeyId = first.key_id().parse().unwrap();
    assert_eq!(id.period.to_string(), "2026-10");
    assert_eq!(first.object().len(), 32);
    assert_ne!(first.object(), second.object());
}
