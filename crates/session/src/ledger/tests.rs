use glave_event::{Fired, Outcome};
use jiff::Timestamp;

use super::*;

fn at(seconds: i64) -> At {
    At::from(Timestamp::from_second(seconds).unwrap())
}

fn project(n: u128) -> Located {
    Located {
        org: glave_ports::OrgId::from_uuid(uuid::Uuid::from_u128(1)),
        project: glave_ports::ProjectId::from_uuid(uuid::Uuid::from_u128(n)),
    }
}

/// A task of a schedule's, not ended: what makes a ledger the scheduler's.
fn running(n: u128) -> Recorded {
    let thread = ThreadId::new(
        Timestamp::UNIX_EPOCH,
        n.to_le_bytes()[..10].try_into().unwrap(),
    );
    Recorded {
        started: TaskStarted {
            thread: thread.unwrap(),
            title: String::new(),
            model: String::new(),
            budget: None,
            schedule: None,
            by: None,
        },
        at: at(0),
        ended: None,
    }
}

/// Reads `projects` as a tick does, each read on from where it was last read: how many were
/// read from their start, every one of them having 10 events.
fn tick(ledgers: &mut Ledgers, projects: std::ops::Range<u128>, scheduled: bool) -> usize {
    let mut from_start = 0;
    for n in projects {
        let ledger = ledgers.kept(project(n));
        if ledger.next == 0 {
            from_start += 1;
            ledger.next = 11;
            if scheduled {
                let task = running(n);
                ledger.open.insert(task.started.thread, task);
            }
        }
    }
    from_start
}

/// Below [`KEPT`] projects, a second tick reads none of their ledgers again from its start: each
/// is read on from where the tick before stopped (#1542).
#[test]
fn a_second_tick_reads_no_ledger_again_from_its_start() {
    let mut ledgers = Ledgers::new();
    let many = 4_096;
    assert_eq!(
        tick(&mut ledgers, 0..many, true),
        4_096,
        "the first reads all"
    );
    assert_eq!(
        tick(&mut ledgers, 0..many, true),
        0,
        "the next reads none again"
    );
    assert_eq!(ledgers.by_read.len(), ledgers.len());
}

#[test]
fn past_every_ledger_kept_the_one_read_longest_ago_is_forgotten() {
    let mut ledgers = Ledgers::new();
    let kept = u128::try_from(KEPT).unwrap();
    tick(&mut ledgers, 0..kept + 1, true);
    assert_eq!(ledgers.len(), KEPT);
    assert!(!ledgers.contains_key(&project(0)), "the first read is gone");
    assert!(ledgers.contains_key(&project(kept)));
}

#[test]
fn a_run_s_end_closes_that_run_alone() {
    let thread = ThreadId::new(Timestamp::UNIX_EPOCH, [1; 10]).unwrap();
    let schedule = ScheduleId::new(Timestamp::UNIX_EPOCH, [2; 10]).unwrap();
    let started = |due| Recorded {
        started: TaskStarted {
            thread,
            title: String::new(),
            model: String::new(),
            budget: None,
            schedule: Some(Fired { schedule, due }),
            by: None,
        },
        at: due,
        ended: None,
    };
    let ended = |due| TaskEnded {
        thread,
        due,
        outcome: Outcome::Completed,
        tokens: glave_event::TokenCounts::default(),
        cost: None,
        by: None,
    };
    let second = started(at(7_200));
    assert!(second.ended_by(&ended(Some(at(7_200)))));
    assert!(
        !second.ended_by(&ended(Some(at(3_600)))),
        "the run before's"
    );
    assert!(
        second.ended_by(&ended(None)),
        "one from before ends carried a run"
    );
}
