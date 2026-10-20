//! The conformance suite: what keys over any [`KeyStorage`] must do. Stores enable the
//! `conformance` feature and run it against their own key storage. It returns the first
//! violation it finds as an error message.

use glave_event::{
    At, CredentialConnected, CredentialId, EventId, Forge, Header, MemoryId, MemorySaved,
    MemoryScope, Outcome, Principal, ProviderConnected, ProviderId, ProviderKind, RawEvent, Secret,
    Seq, ThreadId, TurnEnded, encode,
};
use glave_ports::testing::FAKE_SECRET;
use glave_ports::{Erasable, Eraser, OrgId, ProjectId, Scope};
use uuid::{Builder, Uuid};

use crate::error::KeyError;
use crate::key_id::{KeyId, Period};
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
    Scope::glaves_own(
        OrgId::from_uuid(uuid_v7(9_000)),
        ProjectId::from_uuid(uuid_v7(9_100 + n)),
    )
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

/// A `memory.saved` event for memory `memory`, at position `seq` of the memory ledger.
pub fn memory_event(memory: u64, seq: u64) -> Result<RawEvent, String> {
    let header = Header {
        id: EventId::try_from(uuid_v7(8_000 + seq)).map_err(|e| e.to_string())?,
        thread: ThreadId::try_from(uuid_v7(9_499)).map_err(|e| e.to_string())?,
        seq: Seq::new(seq).map_err(|e| e.to_string())?,
        at: At::try_from("2026-10-11T09:00:00.000Z".to_owned()).map_err(|e| e.to_string())?,
    };
    let saved = MemorySaved {
        memory: MemoryId::try_from(uuid_v7(7_000 + memory)).map_err(|e| e.to_string())?,
        scope: MemoryScope::Project,
        text: format!("remembered thing {memory}"),
        source: None,
        untrusted_read: false,
        by: Principal::glave(),
    };
    encode(header, &saved).map_err(|e| e.to_string())
}

/// Each memory is sealed under a key of its own; destroying it erases that memory alone, and
/// rotating the project key afterwards keeps every other key readable.
async fn memory_keys<S: KeyStorage>(keys: &Keys<S>) -> Verdict {
    let here = scope(8);
    let (first, second) = (memory_event(1, 1)?, memory_event(2, 2)?);
    let month = event(3, 1, "2026-10")?;
    let mut sealed = Vec::new();
    for event in [&first, &second, &month] {
        sealed.push(keys.seal_event(&here, event).await.map_err(text)?);
    }
    let [one, two, plain] = sealed.as_slice() else {
        return Err("three events were sealed".into());
    };
    let id: KeyId = one.key_id.parse().map_err(text)?;
    ensure(
        matches!(id.period, Period::Memory(memory) if memory == uuid_v7(7_001)),
        "a memory's text is sealed under v1/<project>/memory/<memory>/<n>",
    )?;
    ensure(
        one.key_id != two.key_id && plain.key_id.ends_with("/2026-10/1"),
        "each memory has its own key, apart from the month's",
    )?;
    let memory = MemoryId::try_from(uuid_v7(7_001)).map_err(|e| e.to_string())?;
    Eraser::destroy(keys, here, Erasable::Memory(memory))
        .await
        .map_err(text)?;
    ensure(
        matches!(
            open(keys, &here, &first, one).await,
            Err(KeyError::Destroyed(_))
        ),
        "a forgotten memory no longer opens",
    )?;
    ensure(
        keys.rotation_due(&here).await.map_err(text)?,
        "a destroyed memory key leaves a rotation due",
    )?;
    ensure(
        keys.rotate_project_key(&here).await.map_err(text)?,
        "a due rotation rotates",
    )?;
    ensure(
        !keys.rotation_due(&here).await.map_err(text)?,
        "a rotation settles what was due",
    )?;
    ensure(
        open(keys, &here, &second, two).await.is_ok()
            && open(keys, &here, &month, plain).await.is_ok(),
        "other memories and months still open after the rotation",
    )?;
    ensure(
        !keys.rotate_project_key(&here).await.map_err(text)?,
        "nothing to rotate when nothing was destroyed since",
    )
}

/// Each account, a model account or a git host, is sealed under a key of its own; removing it
/// destroys that key, so the account no longer opens, and what else the project holds still
/// does.
async fn account_keys<S: KeyStorage>(keys: &Keys<S>) -> Verdict {
    let here = scope(9);
    let header = |n: u64| -> Result<Header, String> {
        Ok(Header {
            id: EventId::try_from(uuid_v7(n)).map_err(text)?,
            thread: ThreadId::try_from(uuid_v7(9_498)).map_err(text)?,
            seq: Seq::new(n.saturating_sub(8_099)).map_err(text)?,
            at: At::try_from("2026-10-12T09:00:00.000Z".to_owned()).map_err(text)?,
        })
    };
    let model = ProviderConnected {
        provider: ProviderId::try_from(uuid_v7(7_101)).map_err(text)?,
        kind: ProviderKind::Openai,
        url: None,
        key: Secret::new(FAKE_SECRET),
        models: vec!["model".into()],
        by: Principal::glave(),
    };
    let host = CredentialConnected {
        credential: CredentialId::try_from(uuid_v7(7_102)).map_err(text)?,
        forge: Forge::Github,
        url: "https://github.com".into(),
        username: "octocat".into(),
        token: Secret::new(FAKE_SECRET),
        login: None,
        scopes: None,
        renewal: None,
        by: Principal::glave(),
    };
    let accounts = [
        (encode(header(8_100)?, &model).map_err(text)?, 7_101),
        (encode(header(8_101)?, &host).map_err(text)?, 7_102),
    ];
    let month = event(4, 1, "2026-10")?;
    let plain = keys.seal_event(&here, &month).await.map_err(text)?;
    for (account, n) in accounts {
        let sealed = keys.seal_event(&here, &account).await.map_err(text)?;
        let id: KeyId = sealed.key_id.parse().map_err(text)?;
        ensure(
            matches!(id.period, Period::Account(own) if own == uuid_v7(n)),
            "an account is sealed under v1/<project>/provider/<account>/<n>",
        )?;
        ensure(
            !sealed
                .bytes
                .windows(FAKE_SECRET.len())
                .any(|w| w == FAKE_SECRET.as_bytes()),
            "the key or token is sealed, not stored as written",
        )?;
        Eraser::destroy(keys, here, Erasable::Account(uuid_v7(n)))
            .await
            .map_err(text)?;
        let opened = open(keys, &here, &account, &sealed).await;
        ensure(
            matches!(&opened, Err(error) if crate::erased_payload(&sealed.key_id, error).is_some()),
            "a removed account no longer opens, and reads as erased",
        )?;
    }
    ensure(
        open(keys, &here, &month, &plain).await.is_ok(),
        "the month's content still opens",
    )
}

async fn open<S: KeyStorage>(
    keys: &Keys<S>,
    scope: &Scope,
    event: &RawEvent,
    sealed: &crate::keys::Sealed,
) -> Result<Vec<u8>, KeyError> {
    let fields = EventFields::from(event);
    keys.open_event(scope, &fields, &sealed.key_id, &sealed.bytes)
        .await
}

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// Every store's keys must seal and open payloads bound to their rows, keep their numbering
/// through erasure, and make erased content unreadable.
pub async fn keys<S: KeyStorage>(keys: &Keys<S>) -> Verdict {
    sealing(keys).await?;
    erasing_a_project(keys).await?;
    destroying_a_data_key(keys).await?;
    blobs(keys).await?;
    memory_keys(keys).await?;
    account_keys(keys).await?;
    compare_and_swap(keys.storage()).await
}

async fn sealing<S: KeyStorage>(keys: &Keys<S>) -> Verdict {
    let here = scope(1);
    let event = event(1, 1, "2026-10")?;
    let sealed = keys.seal_event(&here, &event).await.map_err(text)?;
    let id: KeyId = sealed.key_id.parse().map_err(text)?;
    ensure(
        id.project == here.project() && id.period.to_string() == "2026-10" && id.n == 1,
        "the first key of a project and month is v1/<project>/<month>/1",
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
    let elsewhere = Scope::glaves_own(here.org(), scope(2).project());
    let crossed = keys
        .open_event(&elsewhere, &fields, &sealed.key_id, &sealed.bytes)
        .await;
    ensure(
        crossed.is_err(),
        "a payload moved to another project must not open",
    )
}

async fn erasing_a_project<S: KeyStorage>(keys: &Keys<S>) -> Verdict {
    let here = scope(3);
    let event = event(1, 1, "2026-10")?;
    let sealed = keys.seal_event(&here, &event).await.map_err(text)?;
    keys.erase_project(&here).await.map_err(text)?;
    let fields = EventFields::from(&event);
    let opened = keys
        .open_event(&here, &fields, &sealed.key_id, &sealed.bytes)
        .await;
    ensure(
        matches!(opened, Err(KeyError::Destroyed(_))),
        "content of an erased project must not open",
    )?;
    let after = keys.seal_event(&here, &event).await.map_err(text)?;
    let id: KeyId = after.key_id.parse().map_err(text)?;
    ensure(
        id.n == 2,
        "after erasure a project gets new keys, never reusing a number",
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
    let before = keys.storage().project(&here).await.map_err(text)?;
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
    let after = keys.storage().project(&here).await.map_err(text)?;
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
        "a project without keys has no blob names",
    )?;
    let mut sealer = keys.blob_sealer(&here).await.map_err(text)?;
    let (object, key_id) = (sealer.object().to_owned(), sealer.key_id().to_owned());
    ensure(
        sealer.write(b"blob").map_err(text)?.is_empty(),
        "a short blob waits for its last chunk",
    )?;
    let finished = sealer.finish().map_err(text)?;
    ensure(
        finished.sha256 == sha,
        "a sealed blob is named by its SHA-256",
    )?;
    let name = keys
        .blob_name(&here, &sha)
        .await
        .map_err(text)?
        .ok_or("sealing a blob makes the project's key")?;
    ensure(
        name != sha.as_str() && name.len() == 64,
        "a blob is stored under an HMAC of its hash, not the hash",
    )?;
    let opened = keys
        .blob_opener(&here, &key_id, &object, &sha)
        .await
        .map_err(text)?
        .open(&finished.last, true)
        .map_err(text)?;
    ensure(opened == b"blob", "a sealed blob must open")?;
    let other = Scope::glaves_own(here.org(), scope(6).project());
    keys.blob_sealer(&other).await.map_err(text)?;
    ensure(
        keys.blob_name(&other, &sha).await.map_err(text)? != Some(name.clone()),
        "the same blob is named differently in another project",
    )?;
    keys.destroy_data_key(
        &here,
        KeyId {
            n: 99,
            ..key_id.parse().map_err(text)?
        },
    )
    .await
    .err()
    .ok_or("destroying an unknown key must fail")?;
    let first: KeyId = key_id.parse().map_err(text)?;
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
        "blob names must survive a project-key rotation",
    )
}

async fn compare_and_swap<S: KeyStorage>(storage: &S) -> Verdict {
    let here = scope(7);
    ensure(
        storage
            .save_project(&here, None, b"one".to_vec())
            .await
            .map_err(text)?,
        "the first save of a project must succeed",
    )?;
    ensure(
        !storage
            .save_project(&here, None, b"two".to_vec())
            .await
            .map_err(text)?,
        "a save expecting no record must fail when one exists",
    )?;
    ensure(
        !storage
            .save_project(&here, Some(7), b"two".to_vec())
            .await
            .map_err(text)?,
        "a save expecting the wrong revision must fail",
    )?;
    ensure(
        storage
            .save_project(&here, Some(1), b"two".to_vec())
            .await
            .map_err(text)?,
        "a save expecting the current revision must succeed",
    )?;
    let stored = storage.project(&here).await.map_err(text)?;
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
