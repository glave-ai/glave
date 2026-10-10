//! The conformance suite: what keys over any [`KeyStorage`] must do. Stores enable the
//! `conformance` feature and run it against their own key storage. It returns the first
//! violation it finds as an error message.

use glave_event::{At, EventId, Header, Outcome, RawEvent, Seq, ThreadId, TurnEnded, encode};
use glave_ports::{OrgId, Scope, SpaceId};
use uuid::{Builder, Uuid};

use crate::error::KeyError;
use crate::key_id::KeyId;
use crate::keys::{EventFields, Keys};
use crate::storage::KeyStorage;

type Verdict = Result<(), String>;

fn ensure(condition: bool, violation: &str) -> Verdict {
    if condition {
        Ok(())
    } else {
        Err(violation.to_owned())
    }
}

fn uuid_v7(n: u64) -> Uuid {
    let mut random = [0u8; 10];
    for (slot, byte) in random.iter_mut().zip(n.to_be_bytes()) {
        *slot = byte;
    }
    Builder::from_unix_timestamp_millis(1_791_000_000_000, &random).into_uuid()
}

fn scope(n: u64) -> Scope {
    Scope {
        org: OrgId::from_uuid(uuid_v7(9_000)),
        space: SpaceId::from_uuid(uuid_v7(9_100 + n)),
    }
}

/// An event of thread `thread`, at position `seq`, in `month` (`YYYY-MM`) of 2026.
pub fn event(thread: u64, seq: u64, month: &str) -> Result<RawEvent, String> {
    let header = Header {
        id: EventId::try_from(uuid_v7(seq)).map_err(|e| e.to_string())?,
        thread: ThreadId::try_from(uuid_v7(9_500 + thread)).map_err(|e| e.to_string())?,
        seq: Seq::new(seq).map_err(|e| e.to_string())?,
        at: At::try_from(format!("{month}-07T21:14:03.512Z")).map_err(|e| e.to_string())?,
    };
    encode(
        header,
        &TurnEnded {
            outcome: Outcome::Completed,
        },
    )
    .map_err(|e| e.to_string())
}

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// Every store's keys must seal and open payloads bound to their rows, keep their numbering
/// through erasure, and make erased content unreadable.
pub async fn keys<S: KeyStorage>(keys: &Keys<S>) -> Verdict {
    sealing(keys).await?;
    erasing_a_space(keys).await?;
    destroying_a_data_key(keys).await?;
    blobs(keys).await?;
    compare_and_swap(keys.storage()).await
}

async fn sealing<S: KeyStorage>(keys: &Keys<S>) -> Verdict {
    let here = scope(1);
    let event = event(1, 1, "2026-10")?;
    let sealed = keys.seal_event(&here, &event).await.map_err(text)?;
    let id: KeyId = sealed.key_id.parse().map_err(text)?;
    ensure(
        id.space == here.space && id.month.to_string() == "2026-10" && id.n == 1,
        "the first key of a space and month is v1/<space>/<month>/1",
    )?;
    ensure(
        !sealed.bytes.windows(9).any(|window| window == b"completed"),
        "sealed bytes must not contain the payload",
    )?;
    let fields = EventFields::from(&event);
    let opened = keys
        .open_event(&here, &fields, &sealed.key_id, &sealed.bytes)
        .await
        .map_err(text)?;
    ensure(
        opened == event.data.get().as_bytes(),
        "a sealed payload must open to exactly what was written",
    )?;
    let again = keys.seal_event(&here, &event).await.map_err(text)?;
    ensure(
        again.key_id == sealed.key_id && again.bytes != sealed.bytes,
        "a month reuses its data key with a fresh nonce",
    )?;
    let moved = [
        EventFields {
            seq: Seq::new(2).map_err(text)?,
            ..fields
        },
        EventFields {
            thread: ThreadId::try_from(uuid_v7(1)).map_err(text)?,
            ..fields
        },
        EventFields {
            kind: "turn.started",
            ..fields
        },
        EventFields { v: 2, ..fields },
        EventFields { glave: 2, ..fields },
    ];
    for fields in moved {
        let opened = keys
            .open_event(&here, &fields, &sealed.key_id, &sealed.bytes)
            .await;
        ensure(
            matches!(opened, Err(KeyError::Tampered(_))),
            "a payload moved to another row must not open",
        )?;
    }
    let elsewhere = Scope {
        org: here.org,
        space: scope(2).space,
    };
    let crossed = keys
        .open_event(&elsewhere, &fields, &sealed.key_id, &sealed.bytes)
        .await;
    ensure(
        crossed.is_err(),
        "a payload moved to another space must not open",
    )
}

async fn erasing_a_space<S: KeyStorage>(keys: &Keys<S>) -> Verdict {
    let here = scope(3);
    let event = event(1, 1, "2026-10")?;
    let sealed = keys.seal_event(&here, &event).await.map_err(text)?;
    keys.erase_space(&here).await.map_err(text)?;
    let fields = EventFields::from(&event);
    let opened = keys
        .open_event(&here, &fields, &sealed.key_id, &sealed.bytes)
        .await;
    ensure(
        matches!(opened, Err(KeyError::Destroyed(_))),
        "content of an erased space must not open",
    )?;
    let after = keys.seal_event(&here, &event).await.map_err(text)?;
    let id: KeyId = after.key_id.parse().map_err(text)?;
    ensure(
        id.n == 2,
        "after erasure a space gets new keys, never reusing a number",
    )?;
    let reopened = keys
        .open_event(&here, &fields, &after.key_id, &after.bytes)
        .await;
    ensure(reopened.is_ok(), "content written after erasure must open")
}

async fn destroying_a_data_key<S: KeyStorage>(keys: &Keys<S>) -> Verdict {
    let here = scope(4);
    let october = event(1, 1, "2026-10")?;
    let november = event(1, 2, "2026-11")?;
    let gone = keys.seal_event(&here, &october).await.map_err(text)?;
    let kept = keys.seal_event(&here, &november).await.map_err(text)?;
    let before = keys.storage().space(&here).await.map_err(text)?;
    keys.destroy_data_key(&here, gone.key_id.parse().map_err(text)?)
        .await
        .map_err(text)?;
    let opened = keys
        .open_event(
            &here,
            &EventFields::from(&october),
            &gone.key_id,
            &gone.bytes,
        )
        .await;
    ensure(
        matches!(opened, Err(KeyError::Destroyed(_))),
        "content under a destroyed data key must not open",
    )?;
    let survived = keys
        .open_event(
            &here,
            &EventFields::from(&november),
            &kept.key_id,
            &kept.bytes,
        )
        .await
        .map_err(|e| format!("other months must survive a rotation: {e}"))?;
    ensure(
        survived == november.data.get().as_bytes(),
        "a surviving payload must open unchanged",
    )?;
    let after = keys.storage().space(&here).await.map_err(text)?;
    ensure(
        after.map(|s| s.revision) > before.map(|s| s.revision),
        "destroying a data key must save a new record",
    )?;
    let resealed = keys.seal_event(&here, &october).await.map_err(text)?;
    let id: KeyId = resealed.key_id.parse().map_err(text)?;
    ensure(
        id.n == 2,
        "a month whose key was destroyed seals new content under its next key",
    )
}

async fn blobs<S: KeyStorage>(keys: &Keys<S>) -> Verdict {
    let here = scope(5);
    let sha = glave_ports::digest(b"blob");
    ensure(
        keys.blob_name(&here, &sha).await.map_err(text)?.is_none(),
        "a space without keys has no blob names",
    )?;
    let (name, sealed) = keys.seal_blob(&here, &sha, b"blob").await.map_err(text)?;
    ensure(
        name != sha.as_str() && name.len() == 64,
        "a blob is stored under an HMAC of its hash, not the hash",
    )?;
    let opened = keys
        .open_blob(&here, &name, &sealed.key_id, &sealed.bytes)
        .await
        .map_err(text)?;
    ensure(opened == b"blob", "a sealed blob must open")?;
    let other = Scope {
        org: here.org,
        space: scope(6).space,
    };
    keys.seal_blob(&other, &sha, b"blob").await.map_err(text)?;
    ensure(
        keys.blob_name(&other, &sha).await.map_err(text)? != Some(name.clone()),
        "the same blob is named differently in another space",
    )?;
    keys.destroy_data_key(
        &here,
        KeyId {
            n: 99,
            ..sealed.key_id.parse().map_err(text)?
        },
    )
    .await
    .err()
    .ok_or("destroying an unknown key must fail")?;
    let first: KeyId = sealed.key_id.parse().map_err(text)?;
    keys.seal_event(&here, &event(1, 1, "2020-01")?)
        .await
        .map_err(text)?;
    let old: KeyId = keys
        .seal_event(&here, &event(1, 1, "2020-01")?)
        .await
        .map_err(text)?
        .key_id
        .parse()
        .map_err(text)?;
    ensure(old != first, "different months use different keys")?;
    keys.destroy_data_key(&here, old).await.map_err(text)?;
    ensure(
        keys.blob_name(&here, &sha).await.map_err(text)? == Some(name),
        "blob names must survive a space-key rotation",
    )
}

async fn compare_and_swap<S: KeyStorage>(storage: &S) -> Verdict {
    let here = scope(7);
    ensure(
        storage
            .save_space(&here, None, b"one".to_vec())
            .await
            .map_err(text)?,
        "the first save of a space must succeed",
    )?;
    ensure(
        !storage
            .save_space(&here, None, b"two".to_vec())
            .await
            .map_err(text)?,
        "a save expecting no record must fail when one exists",
    )?;
    ensure(
        !storage
            .save_space(&here, Some(7), b"two".to_vec())
            .await
            .map_err(text)?,
        "a save expecting the wrong revision must fail",
    )?;
    ensure(
        storage
            .save_space(&here, Some(1), b"two".to_vec())
            .await
            .map_err(text)?,
        "a save expecting the current revision must succeed",
    )?;
    let stored = storage.space(&here).await.map_err(text)?;
    ensure(
        stored.as_ref().map(|s| (s.revision, s.record.as_slice())) == Some((2, b"two".as_slice())),
        "a saved record must read back with its revision",
    )?;
    let org = OrgId::from_uuid(uuid_v7(9_900));
    let first = storage
        .add_org_key(org, b"a".to_vec())
        .await
        .map_err(text)?;
    let second = storage
        .add_org_key(org, b"b".to_vec())
        .await
        .map_err(text)?;
    ensure(
        first == b"a" && second == b"a",
        "an organization keeps its first key",
    )
}
