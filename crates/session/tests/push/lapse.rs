//! A browser kept for a person's notifications lapses 30 days after its browser last
//! subscribed it, unless subscribed again; `push/get` says when to, once less than half of that
//! is left, and subscribing then keeps it 30 days more. Taken out of the organization, it goes
//! at once whatever is left (`removed.rs`).

use super::*;

/// A day, in milliseconds.
const DAY: i64 = 24 * 60 * 60 * 1_000;

/// Whether `listed` says the browser that asked should subscribe again.
fn renew(listed: &[PushDevice]) -> bool {
    listed.iter().any(|device| device.current && device.renew)
}

/// How many records `personal`'s push ledger holds.
async fn recorded(store: &Store, personal: Scope) -> usize {
    let log = store.log();
    log.read(&personal, Reserved::Push.id(), None, 1_000)
        .await
        .unwrap()
        .len()
}

/// Not subscribed again, a browser lapses at 30 days: `push/get` lists it no more, though as a
/// read it writes nothing (`specs/invariants.md`, 5); the next change forgets and erases it.
#[test]
fn a_browser_not_subscribed_again_lapses_at_thirty_days() {
    let dir = folder("push-lapses");
    block_on!(async {
        let setup = Setup::new(Vec::new());
        let clock = setup.clock.clone();
        let (server, store) = pushing_server(setup, &dir).await;
        let me = Caller::default();
        let at = "https://fcm.googleapis.com/fcm/send/lapses";
        subscribe(&server, &me, &Browser::new(13), at).await;
        let personal = chat(&server, &me).await.1;
        clock.advance_millis(30 * DAY - 60_000);
        assert_eq!(
            names(&server, &me).await,
            ["Safari on iPhone"],
            "a minute left"
        );
        clock.advance_millis(60_000);
        let before = recorded(&store, personal).await;
        assert_eq!(names(&server, &me).await, Vec::<String>::new(), "lapsed");
        assert_eq!(
            recorded(&store, personal).await,
            before,
            "the read wrote nothing"
        );
        let other = "https://fcm.googleapis.com/fcm/send/another";
        subscribe(&server, &me, &Browser::new(15), other).await;
        assert_eq!(
            sealed_names(&store, personal).await.len(),
            1,
            "forgotten and erased"
        );
        server.shutdown().await;
    });
}

#[test]
fn subscribing_again_once_half_its_time_is_gone_keeps_it_thirty_days_more() {
    let dir = folder("push-renewed");
    block_on!(async {
        let setup = Setup::new(Vec::new());
        let clock = setup.clock.clone();
        let (server, store) = pushing_server(setup, &dir).await;
        let me = Caller::default();
        let phone = Browser::new(14);
        let at = "https://fcm.googleapis.com/fcm/send/renewed";
        subscribe(&server, &me, &phone, at).await;
        let personal = chat(&server, &me).await.1;
        let added = devices(&server, &me, Some(at)).await[0].added;
        // More than half left: not yet, and subscribing again keeps nothing new.
        clock.advance_millis(14 * DAY);
        assert!(!renew(&devices(&server, &me, Some(at)).await), "day 14");
        subscribe(&server, &me, &phone, at).await;
        assert_eq!(
            sealed_names(&store, personal).await.len(),
            1,
            "nothing new kept"
        );
        // Half gone: subscribing again keeps it 30 days from now, kept once.
        clock.advance_millis(2 * DAY);
        assert!(renew(&devices(&server, &me, Some(at)).await), "day 16");
        subscribe(&server, &me, &phone, at).await;
        assert!(!renew(&devices(&server, &me, Some(at)).await), "renewed");
        assert_eq!(sealed_names(&store, personal).await.len(), 2, "one renewal");
        // Kept past its first 30 days, as first added, until 30 days after the renewal.
        clock.advance_millis(29 * DAY);
        let listed = devices(&server, &me, Some(at)).await;
        assert_eq!(listed.len(), 1, "day 45: kept");
        assert_eq!(listed[0].added, added, "added when it first was");
        clock.advance_millis(DAY);
        assert!(
            devices(&server, &me, None).await.is_empty(),
            "day 46: lapsed"
        );
        server.shutdown().await;
    });
}
