use glave_event::{
    CostKind, CreditsAdded, CreditsCharged, CreditsHeld, CreditsLimitSet, EventId, Header, Nanos,
    Payload, Principal, Seq, ThreadId, encode,
};

use super::*;

/// 2026-10-30T09:00:00Z, and the first of the next month.
const OCTOBER: i64 = 1_793_350_800;
const NOVEMBER: i64 = 1_793_491_200;

fn at(second: i64) -> Timestamp {
    Timestamp::from_second(second).unwrap()
}

fn person(n: u8) -> PrincipalId {
    PrincipalId::new(at(i64::from(n)), [n; 10]).unwrap()
}

/// `book` with `payload` taken in as its next event, written at `second`: the event's id.
fn take<T: Payload>(book: &mut Book, second: i64, payload: &T) -> EventId {
    let [low, high, ..] = book.next_seq().to_le_bytes();
    let id = EventId::new(at(second), [7, 7, 7, 7, 7, 7, 7, 7, high, low]).unwrap();
    let header = Header {
        id,
        thread: ThreadId::reserved(12),
        seq: Seq::new(book.next_seq()).unwrap(),
        at: At::from(at(second)),
    };
    book.apply(encode(header, payload).unwrap());
    id
}

/// Holds `worst` in `book` at `second` as this server does: checked, then written, the call
/// running here.
fn hold(book: &mut Book, worst: i64, second: i64, limit: Option<Money>) -> Result<EventId, Unpaid> {
    book.may_hold(Nanos::new(worst), at(second), limit)?;
    let amount = dollars(Nanos::new(worst));
    let id = take(book, second, &CreditsHeld { amount });
    book.running.insert(id);
    Ok(id)
}

/// `hold`'s call ended costing `cost`, its charge written.
fn release(book: &mut Book, hold: EventId, cost: i64) {
    book.settle(Some(hold), None);
    let charged = CreditsCharged {
        hold: Some(hold),
        ..charged(cost, None)
    };
    take(book, OCTOBER, &charged);
}

fn added(nanos: i64, reference: &str) -> CreditsAdded {
    CreditsAdded {
        amount: dollars(Nanos::new(nanos)),
        reference: reference.into(),
        by: Principal::person(person(9)),
    }
}

fn charged(nanos: i64, who: Option<PrincipalId>) -> CreditsCharged {
    CreditsCharged {
        kind: CostKind::Model,
        amount: dollars(Nanos::new(nanos)),
        person: who,
        hold: None,
    }
}

#[test]
fn the_balance_is_what_was_added_less_what_was_charged_and_each_month_counts_its_own() {
    let (ada, bo) = (person(1), person(2));
    let mut book = Book::default();
    take(&mut book, OCTOBER, &added(1_000_000, "order-1"));
    take(&mut book, OCTOBER, &charged(250_000, Some(ada)));
    take(&mut book, OCTOBER, &charged(100_000, Some(bo)));
    take(&mut book, OCTOBER, &charged(50_000, None));
    assert_eq!(book.added("order-1"), Some(dollars(Nanos::new(1_000_000))));
    assert_eq!(book.added("order-2"), None);
    let october = book.account(at(OCTOBER), None);
    assert_eq!(october.balance, dollars(Nanos::new(600_000)));
    assert_eq!(
        october.spent,
        dollars(Nanos::new(400_000)),
        "everyone's, glave's own too"
    );
    assert_eq!(
        october.people.get(&ada),
        Some(&dollars(Nanos::new(250_000)))
    );
    assert_eq!(october.people.get(&bo), Some(&dollars(Nanos::new(100_000))));
    assert_eq!(october.since.as_second(), 1_790_812_800, "2026-10-01");
    let november = book.account(at(NOVEMBER), None);
    assert_eq!(
        november.balance,
        dollars(Nanos::new(600_000)),
        "the balance carries over"
    );
    assert_eq!(
        november.spent,
        dollars(Nanos::ZERO),
        "the month starts from nothing"
    );
    assert!(november.people.is_empty());
    take(&mut book, NOVEMBER, &charged(10, Some(bo)));
    let november = book.account(at(NOVEMBER), None);
    assert_eq!(
        (november.spent, november.people.len()),
        (dollars(Nanos::new(10)), 1)
    );
}

#[test]
fn a_hold_needs_something_left_of_the_balance_and_of_the_month_s_limit() {
    let mut book = Book::default();
    assert_eq!(
        hold(&mut book, 0, OCTOBER, None),
        Err(Unpaid::Spent),
        "nothing added"
    );
    take(&mut book, OCTOBER, &added(1_000, "order-1"));
    let held = hold(&mut book, 600, OCTOBER, None).unwrap();
    assert_eq!(
        hold(&mut book, 600, OCTOBER, None),
        Err(Unpaid::Short {
            free: dollars(Nanos::new(400)),
            needs: dollars(Nanos::new(600))
        }),
        "what is held counts"
    );
    release(&mut book, held, 0);
    let limit = Some(dollars(Nanos::new(500)));
    assert_eq!(
        hold(&mut book, 600, OCTOBER, limit),
        Err(Unpaid::MonthShort {
            limit: dollars(Nanos::new(500)),
            free: dollars(Nanos::new(500)),
            needs: dollars(Nanos::new(600))
        })
    );
    take(&mut book, OCTOBER, &charged(500, None));
    assert_eq!(
        hold(&mut book, 0, OCTOBER, limit),
        Err(Unpaid::MonthLimit {
            limit: dollars(Nanos::new(500))
        }),
        "this month's limit is reached"
    );
    assert!(hold(&mut book, 400, NOVEMBER, limit).is_ok(), "a new month");
    take(
        &mut book,
        OCTOBER,
        &CreditsLimitSet {
            limit: dollars(Nanos::new(2_000)),
            by: Principal::person(person(9)),
        },
    );
    assert_eq!(
        book.account(at(OCTOBER), limit).limit,
        Some(dollars(Nanos::new(2_000))),
        "the Owner's wins over the server's"
    );
}

#[test]
fn a_charge_counts_at_once_and_its_event_read_back_takes_its_place() {
    let ada = person(1);
    let mut book = Book::default();
    take(&mut book, OCTOBER, &added(1_000, "order-1"));
    let held = hold(&mut book, 700, OCTOBER, None).unwrap();
    assert_eq!(
        hold(&mut book, 400, OCTOBER, None),
        Err(Unpaid::Short {
            free: dollars(Nanos::new(300)),
            needs: dollars(Nanos::new(400))
        }),
        "something is left, but calls running hold most of it: not spent"
    );
    let id = EventId::new(at(OCTOBER), [8; 10]).unwrap();
    let unwritten = Unwritten {
        charged: CreditsCharged {
            hold: Some(held),
            ..charged(300, Some(ada))
        },
        at: At::from(at(OCTOBER)),
        id: Some(id),
    };
    book.settle(Some(held), Some(unwritten.clone()));
    assert!(!book.idle(), "it has a charge to write");
    let counted = book.account(at(OCTOBER), None);
    assert_eq!(
        counted.balance,
        dollars(Nanos::new(700)),
        "counted before it is written"
    );
    assert_eq!(counted.people.get(&ada), Some(&dollars(Nanos::new(300))));
    assert!(
        hold(&mut book, 700, OCTOBER, None).is_ok(),
        "the charge not yet written lets go of the hold here"
    );
    // Its append landed, whoever saw it: read back, it counts once.
    let header = Header {
        id,
        thread: ThreadId::reserved(12),
        seq: Seq::new(book.next_seq()).unwrap(),
        at: unwritten.at,
    };
    book.apply(encode(header, &unwritten.charged).unwrap());
    assert_eq!(book.unwritten.len(), 0);
    assert_eq!(book.account(at(OCTOBER), None), counted);
}

#[test]
fn money_in_another_currency_is_no_part_of_it() {
    let mut book = Book::default();
    let euros = Money {
        nanos: Nanos::new(5_000_000_000),
        currency: Currency::new("EUR").unwrap(),
    };
    take(
        &mut book,
        OCTOBER,
        &CreditsAdded {
            amount: euros,
            ..added(0, "order-1")
        },
    );
    assert_eq!(
        book.account(at(OCTOBER), None).balance,
        dollars(Nanos::ZERO)
    );
}

#[test]
fn a_hold_is_the_most_a_call_could_cost_and_settling_leaves_exactly_what_it_did() {
    let mut book = Book::default();
    take(&mut book, OCTOBER, &added(1_000_000_000, "order-1"));
    let held = hold(&mut book, 1_000_000_000, OCTOBER, None).unwrap();
    assert!(
        hold(&mut book, 1, OCTOBER, None).is_err(),
        "all of it is held"
    );
    release(&mut book, held, 1);
    let account = book.account(at(OCTOBER), None);
    assert_eq!(
        account.balance,
        dollars(Nanos::new(999_999_999)),
        "no cent in it"
    );
    assert_eq!(account.spent, dollars(Nanos::new(1)));
    assert!(book.idle());
}

#[test]
fn at_the_limit_of_what_can_be_counted_a_book_stops_there_and_refuses() {
    let mut book = Book::default();
    take(&mut book, OCTOBER, &added(i64::MAX, "order-1"));
    take(&mut book, OCTOBER, &added(i64::MAX, "order-2"));
    assert_eq!(book.account(at(OCTOBER), None).balance, dollars(Nanos::MAX));
    let held = hold(&mut book, i64::MAX, OCTOBER, None).unwrap();
    assert_eq!(
        hold(&mut book, 1, OCTOBER, None),
        Err(Unpaid::Short {
            free: dollars(Nanos::ZERO),
            needs: dollars(Nanos::new(1))
        }),
        "nothing wraps round to look free"
    );
    release(&mut book, held, 0);
    assert!(book.idle());
    take(&mut book, OCTOBER, &charged(i64::MAX, Some(person(1))));
    take(&mut book, OCTOBER, &charged(i64::MAX, Some(person(1))));
    let account = book.account(at(OCTOBER), None);
    assert_eq!(
        account.spent,
        dollars(Nanos::MAX),
        "spending stops at the most"
    );
    assert_eq!(account.people.get(&person(1)), Some(&dollars(Nanos::MAX)));
    assert_eq!(
        account.balance,
        dollars(Nanos::new(-i64::MAX)),
        "and what is left, far below nothing, is refused"
    );
    assert_eq!(hold(&mut book, 1, OCTOBER, None), Err(Unpaid::Spent));
}

#[test]
fn another_server_s_hold_counts_until_it_lapses_and_this_server_s_own_until_its_call_ends() {
    let mut book = Book::default();
    take(&mut book, OCTOBER, &added(1_000, "order-1"));
    let amount = dollars(Nanos::new(600));
    take(&mut book, OCTOBER, &CreditsHeld { amount });
    assert!(
        hold(&mut book, 600, OCTOBER, None).is_err(),
        "another server's hold counts here"
    );
    let lapsed = OCTOBER + LAPSE.as_secs();
    let mine = hold(&mut book, 600, lapsed, None).unwrap();
    assert!(
        hold(&mut book, 600, lapsed + LAPSE.as_secs(), None).is_err(),
        "a call of this server's own holds while it runs, however long"
    );
    release(&mut book, mine, 0);
    assert!(hold(&mut book, 600, lapsed, None).is_ok());
}
