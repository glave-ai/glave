use std::future::ready;
use std::sync::atomic::{AtomicUsize, Ordering};

use glave_event::{Seq, ThreadId};
use uuid::Uuid;

use super::*;
use crate::block_on;
use crate::scope::{OrgId, ProjectId};
use crate::search::SQUARES_MODULUS;

/// A store's index rows held in a list, counting the loads it serves.
#[derive(Default)]
struct Rows {
    held: Mutex<Vec<(Scope, IndexRow)>>,
    next: Mutex<i64>,
    loads: AtomicUsize,
}

impl Rows {
    fn write(&self, scope: Scope, thread: u128, seq: u64, terms: &[u64]) {
        let mut next = self.next.lock().unwrap();
        *next += 1;
        let row = IndexRow {
            pos: *next,
            found: at(thread, seq),
            terms: terms.to_vec(),
            sealed: 40 + 8 * terms.len() as u64,
        };
        self.held.lock().unwrap().push((scope, row));
    }

    /// Writes a row of one word and `sealed` bytes.
    fn write_sized(&self, scope: Scope, thread: u128, sealed: u64) {
        self.write(scope, thread, 1, &[1]);
        if let Some((_, row)) = self.held.lock().unwrap().last_mut() {
            row.sealed = sealed;
        }
    }

    /// Writes a row at a position before the newest, as a Postgres writer that took its
    /// position first but committed last does.
    fn write_late(&self, scope: Scope, pos: i64, thread: u128, terms: &[u64]) {
        let row = IndexRow {
            pos,
            found: at(thread, 1),
            terms: terms.to_vec(),
            sealed: 48,
        };
        self.held.lock().unwrap().push((scope, row));
    }

    fn delete(&self, thread: u128) {
        let thread = at(thread, 1).thread;
        self.held
            .lock()
            .unwrap()
            .retain(|(_, row)| row.found.thread != thread);
    }

    fn of(&self, scope: &Scope) -> (Fingerprint, Vec<IndexRow>) {
        let mut rows: Vec<IndexRow> = self
            .held
            .lock()
            .unwrap()
            .iter()
            .filter(|(at, _)| at == scope)
            .map(|(_, row)| row.clone())
            .collect();
        rows.sort_by_key(|row| row.pos);
        // The squares as SQL sums them: each reduced, the sum not.
        let print = Fingerprint {
            rows: rows.len() as u64,
            sum: rows.iter().map(|row| row.pos).sum(),
            squares: rows
                .iter()
                .map(|row| (row.pos % SQUARES_MODULUS).pow(2) % SQUARES_MODULUS)
                .sum(),
        };
        (print, rows)
    }
}

impl IndexRows for Rows {
    fn rows(
        &self,
        scope: &Scope,
        after: i64,
        most: u64,
    ) -> impl Future<Output = Result<(Fingerprint, Vec<IndexRow>), LogError>> + Send {
        if after == 0 {
            self.loads.fetch_add(1, Ordering::SeqCst);
        }
        let (print, mut rows) = self.of(scope);
        rows.retain(|row| row.pos > after);
        let mut total = 0;
        let mut kept = Vec::new();
        while let Some(row) = rows.pop() {
            total += row.sealed;
            if total > most {
                break;
            }
            kept.push(row);
        }
        kept.reverse();
        ready(Ok((print, kept)))
    }
}

fn scope(project: u128) -> Scope {
    Scope::glaves_own(
        OrgId::from_uuid(Uuid::from_u128(1)),
        ProjectId::from_uuid(Uuid::from_u128(project)),
    )
}

fn at(thread: u128, seq: u64) -> Found {
    Found {
        thread: ThreadId::try_from(Uuid::from_u128(
            0x0190_0000_0000_7000_8000_0000_0000_0000 | thread,
        ))
        .unwrap(),
        seq: Seq::new(seq).unwrap(),
    }
}

#[test]
fn most_shared_words_come_first_then_the_newest() {
    let rows = Rows::default();
    let here = scope(2);
    rows.write(here, 1, 1, &[10, 11, 12]);
    rows.write(here, 2, 1, &[10]);
    rows.write(here, 3, 1, &[10, 12]);
    rows.write(scope(3), 4, 1, &[10, 11, 12]);
    let cache = SearchCache::new(MOST_CACHED);
    let found = block_on!(cache.search(&rows, &here, (&[10, 11, 12, 12], None), 10)).unwrap();
    assert_eq!(found, [at(1, 1), at(3, 1), at(2, 1)]);
    assert_eq!(
        block_on!(cache.search(&rows, &here, (&[10], None), 2)).unwrap(),
        [at(3, 1), at(2, 1)],
        "at most `limit`, newest first among equals"
    );
    assert_eq!(
        block_on!(cache.search(&rows, &here, (&[99], None), 10)).unwrap(),
        [],
        "no event shares a word with the query"
    );
}

#[test]
fn a_warm_index_reads_only_what_was_written_since() {
    let rows = Rows::default();
    let here = scope(2);
    let cache = SearchCache::new(MOST_CACHED);
    rows.write(here, 1, 1, &[10]);
    assert_eq!(
        block_on!(cache.search(&rows, &here, (&[10], None), 10)).unwrap(),
        [at(1, 1)]
    );
    rows.write(here, 2, 1, &[10]);
    assert_eq!(
        block_on!(cache.search(&rows, &here, (&[10], None), 10)).unwrap(),
        [at(2, 1), at(1, 1)]
    );
    assert_eq!(
        rows.loads.load(Ordering::SeqCst),
        1,
        "loaded once, then caught up"
    );
}

#[test]
fn rows_gone_or_written_out_of_order_load_the_index_again() {
    let rows = Rows::default();
    let here = scope(2);
    let cache = SearchCache::new(MOST_CACHED);
    rows.write(here, 1, 1, &[10]);
    rows.write(here, 2, 1, &[10]);
    block_on!(cache.search(&rows, &here, (&[10], None), 10)).unwrap();
    rows.delete(1);
    rows.write(here, 3, 1, &[10]);
    assert_eq!(
        block_on!(cache.search(&rows, &here, (&[10], None), 10)).unwrap(),
        [at(3, 1), at(2, 1)],
        "a deleted chat's rows leave the index held in memory too"
    );
    assert_eq!(rows.loads.load(Ordering::SeqCst), 2);
    rows.write_late(here, 1, 4, &[10]);
    let late = block_on!(cache.search(&rows, &here, (&[10], None), 10)).unwrap();
    assert!(late.contains(&at(4, 1)), "a row committed late is found");
    assert_eq!(rows.loads.load(Ordering::SeqCst), 3);
}

#[test]
fn the_cache_lets_the_least_recently_searched_go_past_its_size() {
    let rows = Rows::default();
    let words: Vec<u64> = (0..1_000).collect();
    for project in 2..6 {
        rows.write(scope(project), project, 1, &words);
    }
    let one = {
        let alone = SearchCache::new(MOST_CACHED);
        block_on!(alone.search(&rows, &scope(2), (&[1], None), 1)).unwrap();
        alone.bytes()
    };
    let cache = SearchCache::new(one * 2);
    for project in 2..6 {
        block_on!(cache.search(&rows, &scope(project), (&[1], None), 1)).unwrap();
        assert!(
            cache.bytes() <= one * 2,
            "{} over {}",
            cache.bytes(),
            one * 2
        );
    }
    let loads = rows.loads.load(Ordering::SeqCst);
    block_on!(cache.search(&rows, &scope(5), (&[1], None), 1)).unwrap();
    assert_eq!(
        rows.loads.load(Ordering::SeqCst),
        loads,
        "the latest is kept"
    );
    block_on!(cache.search(&rows, &scope(2), (&[1], None), 1)).unwrap();
    assert_eq!(
        rows.loads.load(Ordering::SeqCst),
        loads + 1,
        "the oldest was let go"
    );
}

#[test]
fn an_index_past_its_size_is_loaded_again_newest_first() {
    let rows = Rows::default();
    let here = scope(2);
    let cache = SearchCache::new(MOST_CACHED);
    // Rows of a sealed mebibyte each: a load opens the newest that fit.
    let fit = MOST_LOADED / MIB;
    for n in 0..=fit {
        rows.write_sized(here, u128::from(n), MIB);
    }
    let limit = usize::try_from(fit).unwrap() + 10;
    let found = block_on!(cache.search(&rows, &here, (&[1], None), limit)).unwrap();
    assert_eq!(
        found.len() as u64,
        fit,
        "the oldest is past what one load opens"
    );
    assert!(!found.contains(&at(0, 1)));
    for n in 0..=(MOST_HELD - MOST_LOADED) / MIB {
        rows.write_sized(here, u128::from(fit + 1 + n), MIB);
    }
    let found = block_on!(cache.search(&rows, &here, (&[1], None), limit * 2)).unwrap();
    assert_eq!(found.len() as u64, fit, "past its size, loaded again");
    assert_eq!(rows.loads.load(Ordering::SeqCst), 2);
}

#[test]
fn rare_words_count_for_more_than_common_ones() {
    let rows = Rows::default();
    let here = scope(2);
    // 10 is in every event; 11 in one, older than the rest.
    rows.write(here, 1, 1, &[10, 11]);
    for thread in 2..8 {
        rows.write(here, thread, 1, &[10, 12]);
    }
    let cache = SearchCache::new(MOST_CACHED);
    let found = block_on!(cache.search(&rows, &here, (&[11, 12], None), 1)).unwrap();
    assert_eq!(found, [at(1, 1)], "the rare word outweighs the common one");
}

#[test]
fn a_search_leaves_out_the_thread_asking() {
    let rows = Rows::default();
    let here = scope(2);
    rows.write(here, 1, 1, &[10]);
    for seq in 1..20 {
        rows.write(here, 2, seq, &[10]);
    }
    let cache = SearchCache::new(MOST_CACHED);
    let asking = Some(at(2, 1).thread);
    let found = block_on!(cache.search(&rows, &here, (&[10], asking), 3)).unwrap();
    assert_eq!(
        found,
        [at(1, 1)],
        "its own recent events never crowd out another chat"
    );
}

#[test]
fn a_catch_up_reads_at_most_a_load_then_loads_again() {
    let rows = Rows::default();
    let here = scope(2);
    let cache = SearchCache::new(MOST_CACHED);
    rows.write_sized(here, 0, MIB);
    block_on!(cache.search(&rows, &here, (&[1], None), 1)).unwrap();
    let past = MOST_LOADED / MIB + 2;
    for n in 1..=past {
        rows.write_sized(here, u128::from(n), MIB);
    }
    let limit = usize::try_from(past).unwrap() + 5;
    let found = block_on!(cache.search(&rows, &here, (&[1], None), limit)).unwrap();
    assert_eq!(
        found.len() as u64,
        MOST_LOADED / MIB,
        "loaded again, newest first"
    );
    assert_eq!(found.first(), Some(&at(u128::from(past), 1)));
    assert_eq!(
        rows.loads.load(Ordering::SeqCst),
        2,
        "the cut-off read loaded once more"
    );
}

#[test]
fn rows_gone_and_rows_late_that_keep_the_count_and_sum_still_reload() {
    let rows = Rows::default();
    let here = scope(2);
    let cache = SearchCache::new(MOST_CACHED);
    // Positions 3 and 4 are taken by writers yet to commit.
    for pos in [1, 2, 5, 6, 7] {
        rows.write_late(here, pos, u128::try_from(pos).unwrap(), &[10]);
    }
    block_on!(cache.search(&rows, &here, (&[10], None), 10)).unwrap();
    // 1 and 6 go and 3 and 4 commit: the count and the sum are as they were.
    rows.delete(1);
    rows.delete(6);
    rows.write_late(here, 3, 3, &[10]);
    rows.write_late(here, 4, 4, &[10]);
    let found = block_on!(cache.search(&rows, &here, (&[10], None), 10)).unwrap();
    assert_eq!(found, [at(7, 1), at(5, 1), at(4, 1), at(3, 1), at(2, 1)]);
    assert_eq!(
        rows.loads.load(Ordering::SeqCst),
        2,
        "the squares told them apart"
    );
}
