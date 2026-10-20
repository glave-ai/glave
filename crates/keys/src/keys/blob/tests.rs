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

/// One blob sealed: its object, its key, its sealed chunks (the last included) and what
/// finished.
struct Sealed {
    object: String,
    key_id: String,
    chunks: Vec<Vec<u8>>,
    finished: Finished,
}

/// `content` sealed, given to the sealer in pieces of `piece` bytes.
fn sealed(keys: &Keys<InMemoryKeyStorage>, content: &[u8], piece: usize) -> Sealed {
    let mut sealer = block_on!(keys.blob_sealer(&scope(2))).unwrap();
    let (object, key_id) = (sealer.object().to_owned(), sealer.key_id().to_owned());
    let mut chunks = Vec::new();
    for part in content.chunks(piece.max(1)) {
        chunks.extend(sealer.write(part).unwrap());
    }
    let finished = sealer.finish().unwrap();
    chunks.push(finished.last.clone());
    Sealed {
        object,
        key_id,
        chunks,
        finished,
    }
}

/// `bytes` opened as `object` under `key_id`, fed in pieces of `piece` bytes.
fn opened(
    keys: &Keys<InMemoryKeyStorage>,
    (object, key_id): (&str, &str),
    sha: &Sha256,
    bytes: &[u8],
    piece: usize,
) -> Result<Vec<u8>, KeyError> {
    let mut opener = block_on!(keys.blob_opener(&scope(2), key_id, object, sha))?;
    let mut out = Vec::new();
    for part in bytes.chunks(piece.max(1)) {
        out.extend(opener.feed(part)?);
    }
    out.extend(opener.end()?);
    Ok(out)
}

#[test]
fn a_blob_sealed_and_opened_in_any_pieces_is_whole_in_chunks_of_a_fixed_size() {
    let keys = keys();
    let content: Vec<u8> = (0..3 * CHUNK + 17)
        .map(|n| u8::try_from(n % 251).unwrap())
        .collect();
    for piece in [1000, CHUNK, CHUNK + 1, content.len()] {
        let blob = sealed(&keys, &content, piece);
        assert_eq!(blob.chunks.len(), 4);
        assert!(
            blob.chunks
                .iter()
                .rev()
                .skip(1)
                .all(|chunk| chunk.len() == SEALED_CHUNK)
        );
        assert_eq!(blob.finished.sha256, digest(&content));
        assert_eq!(blob.finished.size, content.len() as u64);
        let bytes = blob.chunks.concat();
        for read in [1, 4096, SEALED_CHUNK, SEALED_CHUNK + 3, bytes.len()] {
            let names = (blob.object.as_str(), blob.key_id.as_str());
            assert_eq!(
                opened(&keys, names, &blob.finished.sha256, &bytes, read).unwrap(),
                content
            );
        }
    }
}

#[test]
fn an_empty_blob_is_one_empty_chunk_and_a_full_last_chunk_stays_last() {
    let keys = keys();
    let full = sealed(&keys, &vec![0; CHUNK], CHUNK);
    assert_eq!(
        full.chunks.len(),
        1,
        "a full chunk waits to learn it is the last"
    );
    assert_eq!(full.finished.last.len(), SEALED_CHUNK);
    let empty = sealed(&keys, b"", 1);
    assert_eq!(empty.chunks.len(), 1);
    let names = (empty.object.as_str(), empty.key_id.as_str());
    assert_eq!(
        opened(&keys, names, &digest(b""), &empty.chunks.concat(), 7).unwrap(),
        b""
    );
}

#[test]
fn bytes_cut_short_reordered_moved_or_of_other_content_do_not_open() {
    let keys = keys();
    let content = vec![7u8; 2 * CHUNK + 5];
    let blob = sealed(&keys, &content, CHUNK);
    let names = (blob.object.as_str(), blob.key_id.as_str());
    let sha = &blob.finished.sha256;
    let tampered = Err(KeyError::Tampered(blob.key_id.clone()));
    let short = blob.chunks.get(..2).unwrap().concat();
    assert_eq!(opened(&keys, names, sha, &short, 100), tampered);
    let bytes = blob.chunks.concat();
    let trimmed = bytes.get(..bytes.len() - 1).unwrap();
    assert_eq!(opened(&keys, names, sha, trimmed, 100), tampered);
    let mut swapped = blob.chunks.clone();
    swapped.swap(0, 1);
    assert_eq!(opened(&keys, names, sha, &swapped.concat(), 100), tampered);
    let other = sealed(&keys, b"x", 1);
    let moved = (other.object.as_str(), blob.key_id.as_str());
    assert_eq!(opened(&keys, moved, sha, &bytes, 100), tampered);
    assert_eq!(
        opened(&keys, names, &digest(b"other"), &bytes, 100),
        tampered,
        "content must have the hash it was asked for"
    );
}

#[test]
fn blobs_are_sealed_under_the_clocks_month_as_random_objects() {
    let keys = keys();
    let (first, second) = (sealed(&keys, b"x", 1), sealed(&keys, b"x", 1));
    let id: KeyId = first.key_id.parse().unwrap();
    assert_eq!(id.period.to_string(), "2026-10");
    assert_eq!(first.object.len(), 32);
    assert_ne!(first.object, second.object);
}
