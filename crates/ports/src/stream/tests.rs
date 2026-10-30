use std::sync::atomic::{AtomicUsize, Ordering};

use glave_event::{At, EventId, Header, ThreadRenamed, encode};
use jiff::Timestamp;
use uuid::Uuid;

use super::*;
use crate::block_on;
use crate::in_memory::InMemoryLog;
use crate::scope::{OrgId, ProjectId};

fn scope() -> Scope {
    Scope::glaves_own(
        OrgId::from_uuid(Uuid::nil()),
        ProjectId::from_uuid(Uuid::nil()),
    )
}

/// Event `seq` of the tasks ledger, titled `title`.
fn event(seq: u64, title: &str) -> RawEvent {
    let header = Header {
        id: EventId::new(Timestamp::UNIX_EPOCH, [1; 10]).unwrap(),
        thread: Reserved::Tasks.id(),
        seq: Seq::new(seq).unwrap(),
        at: At::from(Timestamp::UNIX_EPOCH),
    };
    let renamed = ThreadRenamed {
        title: title.to_owned(),
        by: None,
    };
    encode(header, &renamed).unwrap()
}

/// The `seq` the ledger's next event takes in `log`.
async fn next(log: &InMemoryLog) -> u64 {
    let mut next = 1;
    catch_up(log, &scope(), Reserved::Tasks.id(), None, |page| {
        next += u64::try_from(page.len()).unwrap();
    })
    .await
    .unwrap();
    next
}

#[test]
fn each_ledger_has_its_own_reserved_id() {
    let ids: Vec<String> = Reserved::ALL.iter().map(|l| l.id().to_string()).collect();
    assert_eq!(
        ids.first().map(String::as_str),
        Some("00000000-0000-7000-8000-000000000000")
    );
    assert_eq!(
        ids.last().map(String::as_str),
        Some("00000000-0000-7000-8000-00000000000b")
    );
    for ledger in Reserved::ALL {
        assert_eq!(Reserved::of(ledger.id()), Some(ledger));
    }
    let thread = ThreadId::new(
        Timestamp::UNIX_EPOCH + jiff::SignedDuration::from_secs(1),
        [0; 10],
    );
    assert_eq!(Reserved::of(thread.unwrap()), None);
}

#[test]
fn catching_up_reads_every_page_in_order() {
    let log = InMemoryLog::new();
    let events: Vec<RawEvent> = (1..=PAGE as u64 * 2 + 1)
        .map(|seq| event(seq, "t"))
        .collect();
    block_on!(log.append(&scope(), &events)).unwrap();
    let mut pages = Vec::new();
    let mut seqs = Vec::new();
    block_on!(catch_up(
        &log,
        &scope(),
        Reserved::Tasks.id(),
        None,
        |page| {
            pages.push(page.len());
            seqs.extend(page.iter().map(|raw| raw.seq.get()));
        }
    ))
    .unwrap();
    assert_eq!(pages, [PAGE, PAGE, 1]);
    assert!(seqs.iter().copied().eq(1..=PAGE as u64 * 2 + 1));
    let mut after = Vec::new();
    let from = Seq::new(PAGE as u64 * 2).ok();
    block_on!(catch_up(
        &log,
        &scope(),
        Reserved::Tasks.id(),
        from,
        |page| {
            after.extend(page.iter().map(|raw| raw.seq.get()));
        }
    ))
    .unwrap();
    assert_eq!(after, [PAGE as u64 * 2 + 1]);
}

#[test]
fn an_append_another_writer_beat_is_made_again_after_it() {
    let log = InMemoryLog::new();
    let tries = AtomicUsize::new(0);
    let (log_ref, tries_ref) = (&log, &tries);
    let appended = block_on!(append_retrying::<_, _, StreamError, _>(
        &log,
        &scope(),
        Reserved::Tasks.id(),
        move || async move {
            let seq = next(log_ref).await;
            if tries_ref.fetch_add(1, Ordering::SeqCst) == 0 {
                // Another writer takes this seq first.
                log_ref.append(&scope(), &[event(seq, "theirs")]).await?;
            }
            Ok(Some((vec![event(seq, "ours")], seq)))
        },
    ))
    .unwrap();
    assert_eq!(tries.load(Ordering::SeqCst), 2);
    let (events, seq) = appended.unwrap();
    assert_eq!(seq, 2);
    assert_eq!(events.len(), 1);
    assert_eq!(block_on!(next(&log)), 3);
}

#[test]
fn a_stream_others_keep_moving_is_given_up_on() {
    let log = InMemoryLog::new();
    let tries = AtomicUsize::new(0);
    let (log_ref, tries_ref) = (&log, &tries);
    let given_up = block_on!(append_retrying::<_, (), StreamError, _>(
        &log,
        &scope(),
        Reserved::Tasks.id(),
        move || async move {
            tries_ref.fetch_add(1, Ordering::SeqCst);
            let seq = next(log_ref).await;
            log_ref.append(&scope(), &[event(seq, "theirs")]).await?;
            Ok(Some((vec![event(seq, "ours")], ())))
        },
    ));
    assert_eq!(
        given_up.err(),
        Some(StreamError::Busy {
            thread: Reserved::Tasks.id()
        })
    );

    assert_eq!(tries.load(Ordering::SeqCst), ATTEMPTS);
}

#[test]
fn nothing_made_is_nothing_written() {
    let log = InMemoryLog::new();
    let made = block_on!(append_retrying::<_, (), StreamError, _>(
        &log,
        &scope(),
        Reserved::Tasks.id(),
        || async { Ok(None) },
    ));
    assert!(matches!(made, Ok(None)));
    assert_eq!(block_on!(next(&log)), 1);
}
