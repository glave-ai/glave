use glave_event::{
    EventId, Header, MessageAdded, Origin, OriginKind, Part, Producer, Role, ThreadRenamed,
    ThreadStarted, encode,
};
use uuid::Uuid;

use super::*;

fn header(seq: u64) -> Header {
    let at: At = At::try_from(format!("2026-10-07T21:14:{seq:02}.000Z")).unwrap();
    Header {
        id: EventId::try_from(Uuid::from_u128(
            0x0192_f5c0_11aa_7e21_8c3d_0000_0000_0000 + u128::from(seq),
        ))
        .unwrap(),
        thread: ThreadId::try_from(Uuid::from_u128(0x0192_f5c0_11aa_7e21_8c3d_5f9b_2e4a_6c13))
            .unwrap(),
        seq: Seq::new(seq).unwrap(),
        at,
    }
}

fn started() -> RawEvent {
    let payload = ThreadStarted {
        producer: Producer {
            name: "glave".into(),
            version: "0".into(),
        },
        fork: None,
        parent: None,
        for_: None,
        agent: None,
        origin: Origin::of(OriginKind::Prompt),
    };
    encode(header(1), &payload).unwrap()
}

fn message(seq: u64, role: Role, parts: Vec<Part>) -> RawEvent {
    let payload = MessageAdded {
        role,
        parts,
        author: None,
    };
    encode(header(seq), &payload).unwrap()
}

fn text(text: &str) -> Part {
    Part::Text { text: text.into() }
}

#[test]
fn the_title_is_the_first_line_of_the_first_message_from_the_person() {
    let events = vec![
        started(),
        message(2, Role::Assistant, vec![text("Hello")]),
        message(3, Role::User, vec![]),
        message(
            4,
            Role::User,
            vec![text("\n   \n  Make the build green \nthen ship")],
        ),
        message(5, Role::User, vec![text("Later")]),
    ];
    let view = ThreadView::fold(None, &events).unwrap();
    assert_eq!(view.title_seq, Seq::new(4).ok());
    assert_eq!(
        title_of(&events[3]).as_deref(),
        Some("Make the build green")
    );
    assert_eq!(title_of(&events[1]), None);
}

#[test]
fn the_title_skips_the_instructions_a_thread_opens_with() {
    let first = message(
        2,
        Role::User,
        vec![
            text("# AGENTS.md instructions for /repo\n\n<INSTRUCTIONS>…</INSTRUCTIONS>"),
            text("Say hello"),
        ],
    );
    assert_eq!(title_of(&first).as_deref(), Some("Say hello"));
}

#[test]
fn long_titles_are_cut_with_an_ellipsis() {
    let long = "word ".repeat(40);
    let title = title_of(&message(2, Role::User, vec![text(&long)])).unwrap();
    assert_eq!(title.chars().count(), TITLE_CHARS);
    assert!(title.ends_with("word…"));
}

#[test]
fn a_rename_wins_over_the_first_message_and_later_ones() {
    let renamed = encode(
        header(3),
        &ThreadRenamed {
            title: "Release".into(),
            by: None,
        },
    )
    .unwrap();
    let events = vec![
        started(),
        message(2, Role::User, vec![text("First")]),
        renamed,
    ];
    let view = ThreadView::fold(None, &events).unwrap();
    assert_eq!(view.title_seq, Seq::new(3).ok());
    assert_eq!(title_of(&events[2]).as_deref(), Some("Release"));
}

#[test]
fn folding_is_idempotent_and_needs_a_start() {
    let events = vec![started(), message(2, Role::User, vec![text("Hi")])];
    let once = ThreadView::fold(None, &events).unwrap();
    assert_eq!(ThreadView::fold(Some(once.clone()), &events), Some(once));
    assert_eq!(ThreadView::fold(None, &events[1..]), None);
}

#[test]
fn a_cursor_is_the_views_own_sort_key_ties_broken_by_id() {
    let view = ThreadView::fold(None, &[started()]).unwrap();
    let cursor = view.cursor();
    assert_eq!(
        (cursor.updated_at, cursor.thread),
        (view.updated_at, view.thread)
    );
    let query = |after| ThreadQuery {
        standing: Standing::Active,
        limit: 10,
        after: Some(after),
        open: false,
        parent: None,
    };
    assert!(
        !query(cursor).admits(&view),
        "a page starts after its cursor"
    );
    let mut later = cursor;
    let far = jiff::Timestamp::from_second(4_000_000_000).unwrap();
    later.thread = ThreadId::new(far, [0xff; 10]).unwrap();
    assert!(
        query(later).admits(&view),
        "at the same instant, a lower id comes after"
    );
}
