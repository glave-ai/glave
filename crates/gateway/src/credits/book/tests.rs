use glave_event::{
    CostKind, CreditsAdded, CreditsCharged, CreditsLimitSet, EventId, Header, Nanos, Payload,
    Principal, Seq, ThreadId, encode,
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

/// `book` with `payload` taken in as its next event, written at `second`.
fn take<T: Payload>(book: &mut Book, second: i64, payload: &T) {
    let header = Header {
        id: EventId::new(at(second), [7; 10]).unwrap(),
        thread: ThreadId::reserved(12),
        seq: Seq::new(book.next_seq()).unwrap(),
        at: At::from(at(second)),
    };
    book.apply(encode(header, payload).unwrap());
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
    let month = month_of(at(OCTOBER));
    let mut book = Book::default();
    assert_eq!(
        book.hold(Nanos::new(0), month, None),
        Err(Unpaid::Spent),
        "nothing added"
    );
    take(&mut book, OCTOBER, &added(1_000, "order-1"));
    assert_eq!(book.hold(Nanos::new(600), month, None), Ok(()));
    assert_eq!(
        book.hold(Nanos::new(600), month, None),
        Err(Unpaid::Short {
            free: dollars(Nanos::new(400)),
            needs: dollars(Nanos::new(600))
        }),
        "what is held counts"
    );
    book.settle(Nanos::new(600), None);
    let limit = Some(dollars(Nanos::new(500)));
    assert_eq!(
        book.hold(Nanos::new(600), month, limit),
        Err(Unpaid::MonthShort {
            limit: dollars(Nanos::new(500)),
            free: dollars(Nanos::new(500)),
            needs: dollars(Nanos::new(600))
        })
    );
    take(&mut book, OCTOBER, &charged(500, None));
    assert_eq!(
        book.hold(Nanos::new(0), month, limit),
        Err(Unpaid::MonthLimit {
            limit: dollars(Nanos::new(500))
        }),
        "this month's limit is reached"
    );
    let november = month_of(at(NOVEMBER));
    assert_eq!(
        book.hold(Nanos::new(400), november, limit),
        Ok(()),
        "a new month"
    );
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
    let month = month_of(at(OCTOBER));
    let ada = person(1);
    let mut book = Book::default();
    take(&mut book, OCTOBER, &added(1_000, "order-1"));
    assert_eq!(book.hold(Nanos::new(700), month, None), Ok(()));
    assert_eq!(
        book.hold(Nanos::new(400), month, None),
        Err(Unpaid::Short {
            free: dollars(Nanos::new(300)),
            needs: dollars(Nanos::new(400))
        }),
        "something is left, but calls running hold most of it: not spent"
    );
    let id = EventId::new(at(OCTOBER), [8; 10]).unwrap();
    let unwritten = Unwritten {
        charged: charged(300, Some(ada)),
        at: At::from(at(OCTOBER)),
        id: Some(id),
    };
    book.settle(Nanos::new(700), Some(unwritten.clone()));
    assert!(!book.idle(), "it has a charge to write");
    let counted = book.account(at(OCTOBER), None);
    assert_eq!(
        counted.balance,
        dollars(Nanos::new(700)),
        "counted before it is written"
    );
    assert_eq!(counted.people.get(&ada), Some(&dollars(Nanos::new(300))));
    // Its append landed, whoever saw it: read back, it counts once.
    let header = Header {
        id,
        thread: ThreadId::reserved(12),
        seq: Seq::new(book.next_seq()).unwrap(),
        at: unwritten.at,
    };
    book.apply(encode(header, &unwritten.charged).unwrap());
    assert!(book.idle());
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
    let month = month_of(at(OCTOBER));
    book.hold(Nanos::new(1_000_000_000), month, None).unwrap();
    assert!(
        book.hold(Nanos::new(1), month, None).is_err(),
        "all of it is held"
    );
    let cheap = Unwritten {
        charged: charged(1, None),
        at: At::from(at(OCTOBER)),
        id: None,
    };
    book.settle(Nanos::new(1_000_000_000), Some(cheap));
    let account = book.account(at(OCTOBER), None);
    assert_eq!(
        account.balance,
        dollars(Nanos::new(999_999_999)),
        "no cent in it"
    );
    assert_eq!(account.spent, dollars(Nanos::new(1)));
    assert_eq!(book.held, Nanos::ZERO);
}

#[test]
fn at_the_limit_of_what_can_be_counted_a_book_stops_there_and_refuses() {
    let mut book = Book::default();
    let month = month_of(at(OCTOBER));
    take(&mut book, OCTOBER, &added(i64::MAX, "order-1"));
    take(&mut book, OCTOBER, &added(i64::MAX, "order-2"));
    assert_eq!(book.account(at(OCTOBER), None).balance, dollars(Nanos::MAX));
    book.hold(Nanos::MAX, month, None).unwrap();
    assert_eq!(
        book.hold(Nanos::new(1), month, None),
        Err(Unpaid::Short {
            free: dollars(Nanos::ZERO),
            needs: dollars(Nanos::new(1))
        }),
        "nothing wraps round to look free"
    );
    book.settle(Nanos::MAX, None);
    assert_eq!(book.held, Nanos::ZERO);
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
    assert_eq!(book.hold(Nanos::new(1), month, None), Err(Unpaid::Spent));
}
