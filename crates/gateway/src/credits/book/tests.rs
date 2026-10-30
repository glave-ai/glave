use glave_event::{
    CostKind, CreditsAdded, CreditsCharged, CreditsLimitSet, EventId, Header, Payload, Principal,
    Seq, ThreadId, encode,
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

fn added(micros: i64, reference: &str) -> CreditsAdded {
    CreditsAdded {
        amount: dollars(micros),
        reference: reference.into(),
        by: Principal::person(person(9)),
    }
}

fn charged(micros: i64, who: Option<PrincipalId>) -> CreditsCharged {
    CreditsCharged {
        kind: CostKind::Model,
        amount: dollars(micros),
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
    assert_eq!(book.added("order-1"), Some(dollars(1_000_000)));
    assert_eq!(book.added("order-2"), None);
    let october = book.account(at(OCTOBER), None);
    assert_eq!(october.balance, dollars(600_000));
    assert_eq!(
        october.spent,
        dollars(400_000),
        "everyone's, glave's own too"
    );
    assert_eq!(october.people.get(&ada), Some(&dollars(250_000)));
    assert_eq!(october.people.get(&bo), Some(&dollars(100_000)));
    assert_eq!(october.since.as_second(), 1_790_812_800, "2026-10-01");
    let november = book.account(at(NOVEMBER), None);
    assert_eq!(
        november.balance,
        dollars(600_000),
        "the balance carries over"
    );
    assert_eq!(november.spent, dollars(0), "the month starts from nothing");
    assert!(november.people.is_empty());
    take(&mut book, NOVEMBER, &charged(10, Some(bo)));
    let november = book.account(at(NOVEMBER), None);
    assert_eq!((november.spent, november.people.len()), (dollars(10), 1));
}

#[test]
fn a_hold_needs_something_left_of_the_balance_and_of_the_month_s_limit() {
    let month = month_of(at(OCTOBER));
    let mut book = Book::default();
    assert_eq!(
        book.hold(0, month, None),
        Err(Unpaid::Spent),
        "nothing added"
    );
    take(&mut book, OCTOBER, &added(1_000, "order-1"));
    assert_eq!(book.hold(600, month, None), Ok(()));
    assert_eq!(
        book.hold(600, month, None),
        Err(Unpaid::Short {
            free: dollars(400),
            needs: dollars(600)
        }),
        "what is held counts"
    );
    book.settle(600, None);
    let limit = Some(dollars(500));
    assert_eq!(
        book.hold(600, month, limit),
        Err(Unpaid::MonthShort {
            limit: dollars(500),
            free: dollars(500),
            needs: dollars(600)
        })
    );
    take(&mut book, OCTOBER, &charged(500, None));
    assert_eq!(
        book.hold(0, month, limit),
        Err(Unpaid::MonthLimit {
            limit: dollars(500)
        }),
        "this month's limit is reached"
    );
    let november = month_of(at(NOVEMBER));
    assert_eq!(book.hold(400, november, limit), Ok(()), "a new month");
    take(
        &mut book,
        OCTOBER,
        &CreditsLimitSet {
            limit: dollars(2_000),
            by: Principal::person(person(9)),
        },
    );
    assert_eq!(
        book.account(at(OCTOBER), limit).limit,
        Some(dollars(2_000)),
        "the Owner's wins over the server's"
    );
}

#[test]
fn a_charge_counts_at_once_and_its_event_read_back_takes_its_place() {
    let month = month_of(at(OCTOBER));
    let ada = person(1);
    let mut book = Book::default();
    take(&mut book, OCTOBER, &added(1_000, "order-1"));
    assert_eq!(book.hold(700, month, None), Ok(()));
    assert_eq!(
        book.hold(400, month, None),
        Err(Unpaid::Short {
            free: dollars(300),
            needs: dollars(400)
        }),
        "something is left, but calls running hold most of it: not spent"
    );
    let id = EventId::new(at(OCTOBER), [8; 10]).unwrap();
    let unwritten = Unwritten {
        charged: charged(300, Some(ada)),
        at: At::from(at(OCTOBER)),
        id: Some(id),
    };
    book.settle(700, Some(unwritten.clone()));
    assert!(!book.idle(), "it has a charge to write");
    let counted = book.account(at(OCTOBER), None);
    assert_eq!(
        counted.balance,
        dollars(700),
        "counted before it is written"
    );
    assert_eq!(counted.people.get(&ada), Some(&dollars(300)));
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
        micros: 5_000_000,
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
    assert_eq!(book.account(at(OCTOBER), None).balance, dollars(0));
}
