//! Servers on one store, under the threads' leases (`specs/invariants.md`, 3 and 5): a turn one
//! server runs holds its thread there, another is refused it (`-32011`), and reading the thread
//! anywhere records nothing; a turn a server left open is closed by the next work on its thread,
//! or as the person's server starts, once no live server's lease holds it; a retry with the
//! key a turn was started with, reaching another server, starts no second turn; and work asked
//! of a thread while work that only manages it outlasts every stop takes no lease.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use glave_event::{Body, Outcome, RawEvent, ThreadId, decode};
use glave_ports::testing::FaultyLog;
use glave_ports::{
    BoxFuture, EventLog as _, Holder, InMemoryLeases, Lease, LeaseError, Leased, Leases, OrgId,
};
use glave_session::protocol::{IdempotencyKey, Notification, Reply, Request, code};
use glave_session::{Answering, Repaired, TurnItem, TurnStream};
use tokio::sync::Notify;

use crate::support::*;

/// A server on `log` and `leases` answering `answers`, waiting at `gate` when one is given;
/// with `beside`, the same person on the same projects as that server, as a second process
/// on one store is.
async fn on_store(
    dir: &std::path::Path,
    log: &Shared,
    leases: &Arc<InMemoryLeases>,
    answers: Vec<glave_ports::Completion>,
    beside: Option<&Tested<Shared>>,
    gate: Option<Arc<Notify>>,
) -> Tested<Shared> {
    let mut setup = Setup::asking(answers, Answering::NoOne);
    setup.gate = gate;
    setup.leases = Some(Arc::clone(leases) as Arc<dyn Leases>);
    if let Some(first) = beside {
        setup.seed = 2;
        setup.settings.person = first.person();
        setup.projects = Some(Arc::clone(first.projects()));
    }
    setup.build_on(dir, log.clone()).await.0
}

/// Every event of `thread` the store holds.
async fn stored(log: &Shared, thread: ThreadId) -> Vec<RawEvent> {
    log.read(&scope(), thread, None, usize::MAX).await.unwrap()
}

/// Follows `stream` until its turn has started.
async fn until_started(stream: &mut TurnStream) {
    while let Some(item) = stream.next().await {
        if let TurnItem::Note(note) = item
            && let Notification::Event { event, .. } = note.as_ref()
            && event.kind.as_str() == "turn.started"
        {
            return;
        }
    }
    panic!("the turn never started");
}

/// The reads a client makes of `thread` on `server`: none may record anything.
async fn read_everything(server: &Tested<Shared>, thread: ThreadId) {
    let got = server.request(Request::ThreadGet { thread }).await;
    assert!(matches!(got, Ok(Reply::Thread { .. })), "{got:?}");
    let listed = server
        .request(Request::ThreadList {
            project: Some(scope().project()),
            limit: None,
            show: None,
            cursor: None,
        })
        .await;
    assert!(
        matches!(&listed, Ok(Reply::Threads { threads, .. }) if threads.iter().any(|t| t.id == thread)),
        "{listed:?}"
    );
    let read = server
        .request(Request::ThreadEvents {
            thread,
            after: None,
            limit: None,
        })
        .await;
    assert!(matches!(read, Ok(Reply::Events { .. })), "{read:?}");
}

#[test]
fn a_second_server_reads_a_thread_another_runs_without_writing_and_is_refused_its_turn() {
    let dir = folder("leases-elsewhere");
    block_on!(async {
        let (log, (_, leases)) = (Shared::default(), shared_leases());
        let gate = Arc::new(Notify::new());
        let slow = vec![answer(Some("Slow."), None)];
        let first = on_store(&dir, &log, &leases, slow, None, Some(Arc::clone(&gate))).await;
        let mine = vec![answer(Some("Mine."), None)];
        let second = on_store(&dir, &log, &leases, mine, Some(&first), None).await;
        let thread = start(&first, false).await;
        let mut running = stream_turn(&first, thread, "slow", None).await.unwrap();
        until_started(&mut running).await;

        let before = stored(&log, thread).await;
        read_everything(&second, thread).await;
        let refused = turn(&second, thread, "mine").await;
        assert_eq!(
            refused.err().map(|e| e.code),
            Some(code::RUNNING_ELSEWHERE),
            "the first server holds the thread"
        );
        assert_eq!(
            ids(&stored(&log, thread).await),
            ids(&before),
            "reading recorded nothing"
        );

        gate.notify_one();
        assert_eq!(running.finish().await.unwrap().outcome, Outcome::Completed);
        let reply = turn(&second, thread, "mine").await.unwrap();
        assert!(matches!(reply, Reply::Turn { turn } if turn.outcome == Outcome::Completed));
    });
}

/// A thread whose turn a server left open in the model call, as the store has it once that
/// server is gone: copied into a log of its own, so nothing of the first server writes to it
/// again. The first server's person and projects come with it. Its turn was sent with `key`.
async fn left_open(
    dir: &std::path::Path,
    key: Option<IdempotencyKey>,
) -> (Shared, Tested<Shared>, ThreadId) {
    let (gone, (_, its_leases)) = (Shared::default(), shared_leases());
    let gate = Arc::new(Notify::new());
    let answers = vec![answer(Some("Slow."), None)];
    let first = on_store(dir, &gone, &its_leases, answers, None, Some(gate)).await;
    let thread = start(&first, false).await;
    let mut running = stream_turn(&first, thread, "slow", key).await.unwrap();
    until_started(&mut running).await;
    let log = Shared::default();
    log.append(&scope(), &stored(&gone, thread).await)
        .await
        .unwrap();
    (log, first, thread)
}

/// The ids of `events`, in order: what tells whether anything was recorded.
fn ids(events: &[RawEvent]) -> Vec<glave_event::EventId> {
    events.iter().map(|event| event.id).collect()
}

/// The kinds of `events`, in order.
fn kinds_of(events: &[RawEvent]) -> Vec<&str> {
    events.iter().map(|event| event.kind.as_str()).collect()
}

#[test]
fn a_turn_left_open_is_closed_as_the_server_starts_once_no_live_lease_holds_it() {
    let dir = folder("leases-left-open");
    block_on!(async {
        let (log, first, thread) = left_open(&dir, None).await;
        let (clock, leases) = shared_leases();
        // The server that left it stopped less than a lease ago.
        let gone = Holder::from_uuid(uuid::Uuid::from_u128(99));
        let lapsing = Duration::from_secs(30);
        let on = Leased::Thread(thread);
        let held = leases.acquire(scope().org(), on, gone, lapsing).await;
        assert!(held.unwrap().is_some());
        let answers = vec![answer(Some("Again."), None)];
        let second = on_store(&dir, &log, &leases, answers, Some(&first), None).await;

        let before = stored(&log, thread).await;
        read_everything(&second, thread).await;
        assert_eq!(
            ids(&stored(&log, thread).await),
            ids(&before),
            "reading recorded nothing"
        );
        let repaired = second.repair_left_open().await.unwrap();
        assert_eq!(repaired, Repaired { closed: 0, held: 1 });
        let refused = turn(&second, thread, "again").await;
        assert_eq!(refused.err().map(|e| e.code), Some(code::RUNNING_ELSEWHERE));
        assert_eq!(ids(&stored(&log, thread).await), ids(&before));

        clock.advance_seconds(31);
        let repaired = second.repair_left_open().await.unwrap();
        assert_eq!(repaired, Repaired { closed: 1, held: 0 });
        let events = stored(&log, thread).await;
        assert_eq!(
            kinds_of(&events),
            [kinds_of(&before).as_slice(), &["turn.ended"]].concat()
        );
        let ended = decode(events.last().unwrap().clone()).unwrap().body;
        assert!(matches!(ended, Body::TurnEnded(ended) if ended.outcome == Outcome::Interrupted));
        assert_eq!(
            second.repair_left_open().await.unwrap(),
            Repaired::default()
        );
        let reply = turn(&second, thread, "again").await.unwrap();
        assert!(matches!(reply, Reply::Turn { turn } if turn.outcome == Outcome::Completed));
    });
}

#[test]
fn work_on_a_thread_left_open_closes_it_first() {
    let dir = folder("leases-work-closes");
    block_on!(async {
        let (log, first, thread) = left_open(&dir, None).await;
        let (_, leases) = shared_leases();
        let answers = vec![answer(Some("Again."), None)];
        let second = on_store(&dir, &log, &leases, answers, Some(&first), None).await;
        let before = stored(&log, thread).await;
        let reply = turn(&second, thread, "again").await.unwrap();
        assert!(matches!(reply, Reply::Turn { turn } if turn.outcome == Outcome::Completed));
        let events = stored(&log, thread).await;
        let after: Vec<&str> = kinds_of(&events)[before.len()..].to_vec();
        assert_eq!(after.first(), Some(&"turn.ended"), "{after:?}");
        assert_eq!(after.get(1), Some(&"turn.started"), "{after:?}");
        let closed = decode(events[before.len()].clone()).unwrap().body;
        assert!(matches!(closed, Body::TurnEnded(ended) if ended.outcome == Outcome::Interrupted));
    });
}

/// How many turns `events` start.
fn turns_started(events: &[RawEvent]) -> usize {
    kinds_of(events)
        .into_iter()
        .filter(|kind| *kind == "turn.started")
        .count()
}

#[test]
fn a_retry_reaching_another_server_is_answered_as_its_turn_ended_and_starts_none() {
    let dir = folder("leases-retry-elsewhere");
    block_on!(async {
        let (log, (_, leases)) = (Shared::default(), shared_leases());
        let first = on_store(
            &dir,
            &log,
            &leases,
            vec![answer(Some("Hi."), None)],
            None,
            None,
        )
        .await;
        let answers = vec![answer(Some("Other."), None)];
        let second = on_store(&dir, &log, &leases, answers, Some(&first), None).await;
        let thread = start(&first, false).await;
        let key = IdempotencyKey::new("attempt-1").unwrap();
        let mut sent = stream_turn(&first, thread, "hi", Some(key.clone()))
            .await
            .unwrap();
        let ended = sent.finish().await.unwrap();
        assert_eq!(ended.outcome, Outcome::Completed);

        let mut retried = stream_turn(&second, thread, "hi", Some(key)).await.unwrap();
        assert_eq!(
            retried.finish().await.unwrap(),
            ended,
            "answered as it ended"
        );
        assert_eq!(
            turns_started(&stored(&log, thread).await),
            1,
            "no second turn"
        );

        let other = IdempotencyKey::new("attempt-2").unwrap();
        let mut another = stream_turn(&second, thread, "more", Some(other))
            .await
            .unwrap();
        assert_eq!(another.finish().await.unwrap().outcome, Outcome::Completed);
        assert_eq!(
            turns_started(&stored(&log, thread).await),
            2,
            "another key starts one"
        );
    });
}

#[test]
fn a_retry_after_its_server_stopped_mid_turn_closes_that_turn_and_starts_none() {
    let dir = folder("leases-retry-left-open");
    block_on!(async {
        let key = IdempotencyKey::new("attempt-1").unwrap();
        let (log, first, thread) = left_open(&dir, Some(key.clone())).await;
        let (_, leases) = shared_leases();
        let answers = vec![answer(Some("Again."), None)];
        let second = on_store(&dir, &log, &leases, answers, Some(&first), None).await;
        let mut retried = stream_turn(&second, thread, "slow", Some(key))
            .await
            .unwrap();
        let ended = retried.finish().await.unwrap();
        assert_eq!(
            ended.outcome,
            Outcome::Interrupted,
            "as its turn was closed"
        );
        let events = stored(&log, thread).await;
        assert_eq!(turns_started(&events), 1, "no second turn");
        assert_eq!(kinds_of(&events).last(), Some(&"turn.ended"));
    });
}

/// Leases in memory that count the times one is taken.
struct Counted(Arc<InMemoryLeases>, AtomicUsize);

#[deny(clippy::missing_trait_methods)]
impl Leases for Counted {
    fn acquire(
        &self,
        org: OrgId,
        on: Leased,
        holder: Holder,
        ttl: Duration,
    ) -> BoxFuture<'_, Result<Option<Lease>, LeaseError>> {
        self.1.fetch_add(1, Ordering::SeqCst);
        self.0.acquire(org, on, holder, ttl)
    }

    fn renew<'a>(
        &'a self,
        lease: &'a Lease,
        ttl: Duration,
    ) -> BoxFuture<'a, Result<Option<Lease>, LeaseError>> {
        self.0.renew(lease, ttl)
    }

    fn release<'a>(&'a self, lease: &'a Lease) -> BoxFuture<'a, Result<(), LeaseError>> {
        self.0.release(lease)
    }
}

/// A turn asked for while work that only manages its thread outlasts every stop (a rename,
/// which no stop ends, whose record the store holds) is refused, taking no lease: one taken
/// then would only renew the rename's, which it lets go of as it ends, leaving the turn
/// running under none, open to another server.
#[test]
fn a_turn_asked_while_managing_work_outlasts_every_stop_takes_no_lease() {
    let dir = folder("leases-outlasted");
    block_on!(async {
        let counted = Arc::new(Counted(shared_leases().1, AtomicUsize::new(0)));
        let mut setup = Setup::new(vec![answer(Some("Hi."), None)]);
        setup.leases = Some(Arc::clone(&counted) as Arc<dyn Leases>);
        let log = FaultyLog::default();
        let (server, _) = setup.build_on(&dir, log.clone()).await;
        let thread = start(&server, false).await;
        log.hold("thread.renamed");
        let title = "Notes".to_owned();
        let renaming = server.request(Request::ThreadRename { thread, title });
        let asking = async {
            log.held().await;
            let taken = counted.1.load(Ordering::SeqCst);
            let refused = stream_turn(&server, thread, "hi", None).await;
            let after = counted.1.load(Ordering::SeqCst);
            log.release();
            (refused.err().map(|error| error.code), after - taken)
        };
        let (renamed, (refused, leases)) = tokio::join!(renaming, asking);
        assert!(renamed.is_ok(), "{renamed:?}");
        assert_eq!(refused, Some(code::TURN_RUNNING));
        assert_eq!(leases, 0, "no lease taken while the rename held the thread");
    });
}
