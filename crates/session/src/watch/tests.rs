use std::collections::BTreeMap;
use std::sync::Mutex;

use glave_event::{At, EventId, Header, ThreadDeleted, ThreadRenamed, encode};
use jiff::Timestamp;

use super::*;
use crate::hub::Hub;
use crate::protocol::Update;

pub(crate) fn thread(n: u8) -> ThreadId {
    ThreadId::new(Timestamp::UNIX_EPOCH, [n; 10]).unwrap()
}

fn at() -> At {
    At::try_from("2026-10-01T00:00:00.000Z".to_owned()).unwrap()
}

/// The `seq`th event of `thread`: a rename, or with `deleted` its deletion.
pub(crate) fn event(thread: ThreadId, seq: u64, deleted: bool) -> RawEvent {
    let mut random = [0; 10];
    random[..8].copy_from_slice(&seq.to_be_bytes());
    random[9] = thread.as_uuid().as_bytes()[15];
    let header = Header {
        id: EventId::new(Timestamp::UNIX_EPOCH, random).unwrap(),
        thread,
        seq: Seq::new(seq).unwrap(),
        at: at(),
    };
    if deleted {
        encode(header, &ThreadDeleted { by: None }).unwrap()
    } else {
        let title = format!("title {seq}");
        encode(header, &ThreadRenamed { title, by: None }).unwrap()
    }
}

/// A log of threads' events a watch reads, and the hub it hears them from.
#[derive(Default)]
struct World {
    log: Arc<Mutex<BTreeMap<ThreadId, Vec<RawEvent>>>>,
    hub: Hub,
}

impl World {
    /// Records `thread`'s events up to `seq`, each `told` to watchers as it is recorded.
    fn record(&self, thread: ThreadId, through: u64, told: bool) {
        for seq in self.last(thread) + 1..=through {
            let recorded = event(thread, seq, false);
            self.log
                .lock()
                .unwrap()
                .entry(thread)
                .or_default()
                .push(recorded.clone());
            if told {
                self.tell(&recorded);
            }
        }
    }

    fn last(&self, thread: ThreadId) -> u64 {
        let log = self.log.lock().unwrap();
        log.get(&thread)
            .and_then(|events| events.last())
            .map_or(0, |event| event.seq.get())
    }

    /// Tells watchers of `event`, recorded or not.
    fn tell(&self, event: &RawEvent) {
        let note = Notification::Event {
            thread: event.thread,
            event: event.clone(),
        };
        self.hub.publish(event.thread, note);
    }

    fn live(&self, thread: ThreadId, text: &str) {
        let update = Update::Text { text: text.into() };
        self.hub
            .publish(thread, Notification::Live { thread, update });
    }

    fn reader(&self) -> Reader {
        let log = Arc::clone(&self.log);
        Arc::new(move |thread, after| {
            let after = after.map_or(0, Seq::get);
            let page: Vec<RawEvent> = log
                .lock()
                .unwrap()
                .get(&thread)
                .into_iter()
                .flatten()
                .filter(|event| event.seq.get() > after)
                .take(PAGE)
                .cloned()
                .collect();
            Box::pin(async move { Ok(page) })
        })
    }

    fn watch(&self, watched: ThreadId, after: Option<u64>) -> Watch {
        let after = after.map(|after| Seq::new(after).unwrap());
        Watch::new(watched, self.reader(), self.hub.watch(watched), after)
    }
}

/// What a watch says, until nothing more has come: each event as `thread:seq`, each live
/// update as its text, and its end.
async fn said(watch: &mut Watch) -> Vec<String> {
    let mut said = Vec::new();
    while let Some(item) = watch.item(false).await {
        said.push(match item {
            Watched::Note(note) => match note.as_ref() {
                Notification::Event { thread, event } => {
                    let n = thread.as_uuid().as_bytes()[15];
                    format!("{n}:{}", event.seq.get())
                }
                Notification::Live {
                    update: Update::Text { text },
                    ..
                } => text.clone(),
                other => format!("{other:?}"),
            },
            Watched::Ended(ended) => format!("ended {ended:?}"),
        });
    }
    said
}

fn seqs(thread: u8, seqs: std::ops::RangeInclusive<u64>) -> Vec<String> {
    seqs.map(|seq| format!("{thread}:{seq}")).collect()
}

#[test]
fn the_log_comes_first_then_each_event_as_recorded_with_live_updates_between() {
    glave_ports::block_on!(async {
        let world = World::default();
        let watched = thread(1);
        world.record(watched, 2, false);
        let mut watch = world.watch(watched, None);
        assert_eq!(said(&mut watch).await, seqs(1, 1..=2));
        world.record(watched, 3, true);
        world.live(watched, "hi");
        world.record(watched, 4, true);
        let mut want = seqs(1, 3..=3);
        want.push("hi".into());
        want.extend(seqs(1, 4..=4));
        assert_eq!(said(&mut watch).await, want);
    });
}

#[test]
fn what_is_recorded_while_the_log_is_read_comes_once_and_older_progress_never() {
    glave_ports::block_on!(async {
        let world = World::default();
        let watched = thread(1);
        world.record(watched, 1, false);
        let mut watch = world.watch(watched, None);
        // Recorded after the watch began and before it read: both read and heard.
        world.record(watched, 2, true);
        world.live(watched, "older than 3");
        world.record(watched, 3, true);
        world.live(watched, "fresh");
        let mut want = seqs(1, 1..=3);
        want.push("fresh".into());
        assert_eq!(said(&mut watch).await, want);
    });
}

#[test]
fn progress_flows_at_once_when_what_was_read_had_been_published_before() {
    glave_ports::block_on!(async {
        let world = World::default();
        let watched = thread(1);
        // Published before the watch began, with no one watching: no copy of it is to come.
        world.record(watched, 2, true);
        let mut watch = world.watch(watched, None);
        world.live(watched, "streaming");
        let mut want = seqs(1, 1..=2);
        want.push("streaming".into());
        assert_eq!(said(&mut watch).await, want);
    });
}

#[test]
fn after_skips_what_the_client_has() {
    glave_ports::block_on!(async {
        let world = World::default();
        let watched = thread(1);
        world.record(watched, 5, false);
        let mut watch = world.watch(watched, Some(3));
        world.record(watched, 6, true);
        assert_eq!(said(&mut watch).await, seqs(1, 4..=6));
    });
}

#[test]
fn an_event_ahead_of_what_was_sent_has_the_rest_read_from_the_log() {
    glave_ports::block_on!(async {
        let world = World::default();
        let watched = thread(1);
        world.record(watched, 1, false);
        let mut watch = world.watch(watched, None);
        assert_eq!(said(&mut watch).await, seqs(1, 1..=1));
        // Recorded in order, but told out of it, as two writers' appends may be.
        world.record(watched, 3, false);
        world.tell(&event(watched, 3, false));
        world.tell(&event(watched, 2, false));
        assert_eq!(said(&mut watch).await, seqs(1, 2..=3));
    });
}

#[test]
fn a_watcher_pushed_past_its_backlog_gets_every_event_from_the_log_and_no_stale_progress() {
    glave_ports::block_on!(async {
        let world = World::default();
        let watched = thread(1);
        let mut watch = world.watch(watched, None);
        for seq in 1..=1_500 {
            world.live(watched, "stale");
            world.record(watched, seq, true);
        }
        world.live(watched, "stale too");
        world.record(watched, 1_501, true);
        world.live(watched, "fresh");
        let mut want = seqs(1, 1..=1_501);
        want.push("fresh".into());
        assert_eq!(said(&mut watch).await, want);
    });
}

#[test]
fn a_subagent_first_seen_part_way_is_read_from_its_start() {
    glave_ports::block_on!(async {
        let world = World::default();
        let (parent, child) = (thread(1), thread(2));
        world.record(parent, 1, false);
        let mut watch = world.watch(parent, None);
        world.record(child, 3, false);
        // A child's notifications reach its parent's watchers.
        world.hub.publish(
            parent,
            Notification::Event {
                thread: child,
                event: event(child, 3, false),
            },
        );
        let mut want = seqs(1, 1..=1);
        want.extend(seqs(2, 1..=3));
        assert_eq!(said(&mut watch).await, want);
    });
}

#[test]
fn deleting_the_thread_ends_the_watch_once_it_is_sent() {
    glave_ports::block_on!(async {
        let world = World::default();
        let watched = thread(1);
        world.record(watched, 1, false);
        let mut watch = world.watch(watched, None);
        let deleted = event(watched, 2, true);
        world
            .log
            .lock()
            .unwrap()
            .get_mut(&watched)
            .unwrap()
            .push(deleted.clone());
        world.tell(&deleted);
        world.record(watched, 3, true);
        let mut want = seqs(1, 1..=2);
        want.push("ended Ok(Deleted)".into());
        assert_eq!(said(&mut watch).await, want);
        assert!(watch.next().await.is_none());
    });
}

/// Tells the watchers of `parent` of `event`, a subagent's.
fn tell_child(world: &World, parent: ThreadId, event: &RawEvent) {
    let note = Notification::Event {
        thread: event.thread,
        event: event.clone(),
    };
    world.hub.publish(parent, note);
}

fn child_live(world: &World, parent: ThreadId, child: ThreadId, text: &str) {
    let update = Update::Text { text: text.into() };
    world.hub.publish(
        parent,
        Notification::Live {
            thread: child,
            update,
        },
    );
}

#[test]
fn after_falling_behind_each_threads_progress_waits_for_its_own_events_only() {
    glave_ports::block_on!(async {
        let world = World::default();
        let (parent, a, b) = (thread(1), thread(2), thread(3));
        world.record(parent, 1, false);
        let mut watch = world.watch(parent, None);
        for child in [a, b] {
            world.record(child, 1, false);
            tell_child(&world, parent, &event(child, 1, false));
        }
        let mut want = seqs(1, 1..=1);
        want.extend(seqs(2, 1..=1));
        want.extend(seqs(3, 1..=1));
        assert_eq!(said(&mut watch).await, want);
        for _ in 0..1_100 {
            world.live(parent, "missed");
        }
        for child in [a, b] {
            world.record(child, 2, false);
        }
        tell_child(&world, parent, &event(a, 2, false));
        child_live(&world, parent, b, "older than b's second");
        tell_child(&world, parent, &event(b, 2, false));
        child_live(&world, parent, b, "b fresh");
        child_live(&world, parent, a, "a fresh");
        let mut want = seqs(2, 2..=2);
        want.extend(seqs(3, 2..=2));
        want.extend(["b fresh".to_owned(), "a fresh".to_owned()]);
        assert_eq!(said(&mut watch).await, want);
    });
}

#[test]
fn past_the_most_subagents_the_one_sent_to_longest_ago_is_forgotten() {
    let world = World::default();
    let watched = thread(1);
    let mut watch = world.watch(watched, Some(1));
    let child = |n: u16| {
        let mut random = [7; 10];
        random[..2].copy_from_slice(&n.to_be_bytes());
        ThreadId::new(Timestamp::UNIX_EPOCH, random).unwrap()
    };
    for n in 0..=300 {
        watch.keep_place(child(n), Seq::FIRST);
    }
    assert_eq!(watch.sent.len(), MOST_CHILDREN + 1);
    assert!(watch.sent.contains_key(&watched));
    // The oldest child, seen again, is kept, and the one sent to longest ago goes instead.
    watch.keep_place(child(0), Seq::FIRST);
    assert!(watch.sent.contains_key(&child(0)));
    assert!(!watch.sent.contains_key(&child(45)));
    assert!(watch.sent.contains_key(&child(46)));
    assert_eq!(watch.sent.len(), MOST_CHILDREN + 1);
}
