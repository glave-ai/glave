use std::sync::Arc;

use glave_ports::testing::ManualClock;
use glave_ports::{OrgId, ProjectId, Scope, block_on};
use glave_store_sqlite::{SqliteLog, Store};

use super::*;
use crate::testing::Authenticator;

const START: i64 = 1_791_590_400;
const ORIGIN: &str = "https://glave.test";
const SERVER: &str = "0192f5c6-0000-7000-8000-0000000005e7";

/// The person's passkey handle, which the caller keeps.
const HANDLE: Uuid = Uuid::from_u128(0x0dd1_7e5a_5e7a_4a1d_8a2e_0000_0000_0691);

fn store() -> Store {
    let root = glave_keys::InMemoryRoot::new().unwrap();
    Store::open_in_memory(glave_keys::KeySettings::new(Arc::new(root))).unwrap()
}

fn scope() -> Scope {
    Scope::glaves_own(
        OrgId::from_uuid(Uuid::from_u128(1)),
        ProjectId::from_uuid(Uuid::from_u128(2)),
    )
}

/// The browsers of a server reached at [`ORIGIN`], on `log`.
async fn open(log: SqliteLog, clock: &ManualClock) -> Browsers {
    open_for(log, scope(), HANDLE, clock).await
}

/// The browsers of the person whose personal project is `scope` and whose handle is
/// `handle`, on a server reached at [`ORIGIN`], on `log`.
async fn open_for(log: SqliteLog, scope: Scope, handle: Uuid, clock: &ManualClock) -> Browsers {
    let origin = PublicOrigin::parse(ORIGIN).unwrap();
    Browsers::open(
        log,
        scope,
        SERVER.into(),
        handle,
        Some(&origin),
        Box::new(clock.clone()),
    )
    .await
    .unwrap()
}

/// A passkey made on `authenticator` through a new setup link, and the session it began.
async fn set_up(browsers: &Browsers, authenticator: &mut Authenticator) -> (PasskeyInfo, Session) {
    let code = browsers.setup().unwrap();
    let asked = browsers.create_options(Proof::Code(code.expose())).unwrap();
    let answer = authenticator.create(&asked.options).unwrap();
    browsers
        .create(&asked.id, &answer, Some("Phone"))
        .await
        .unwrap()
}

/// A browser signed in with `authenticator`'s passkey.
async fn sign_in(
    browsers: &Browsers,
    authenticator: &mut Authenticator,
) -> Result<Session, BrowserError> {
    let asked = browsers.get_options()?;
    let answer = authenticator.get(&asked.options).unwrap();
    browsers.get(&asked.id, &answer).await
}

#[test]
fn a_setup_link_makes_a_passkey_once_and_signs_that_browser_in() {
    block_on!(async {
        let clock = ManualClock::at_second(START);
        let browsers = open(store().log(), &clock).await;
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        let code = browsers.setup().unwrap();
        let asked = browsers.create_options(Proof::Code(code.expose())).unwrap();
        assert_eq!(asked.options["publicKey"]["rp"]["id"], "glave.test");
        let answer = phone.create(&asked.options).unwrap();
        let (passkey, session) = browsers.create(&asked.id, &answer, None).await.unwrap();
        assert_eq!(passkey.name, "Passkey added 2026-10-10");
        assert_eq!(
            browsers.verify(session.token.expose(), true).await.unwrap(),
            session.id
        );
        assert!(matches!(
            browsers.create_options(Proof::Code(code.expose())),
            Err(BrowserError::Setup)
        ));
        assert!(matches!(
            browsers.create(&asked.id, &answer, None).await,
            Err(BrowserError::Ceremony)
        ));
    });
}

#[test]
fn a_passkey_signs_a_browser_in_across_a_restart_and_another_origin_s_answer_is_refused() {
    block_on!(async {
        let clock = ManualClock::at_second(START);
        let store = store();
        let browsers = open(store.log(), &clock).await;
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        set_up(&browsers, &mut phone).await;
        let session = sign_in(&browsers, &mut phone).await.unwrap();
        assert!(browsers.verify(session.token.expose(), true).await.is_ok());

        let reopened = open(store.log(), &clock).await;
        let listed = reopened.passkeys();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed.first().unwrap().name, "Phone");
        assert!(listed.first().unwrap().used.is_some());
        assert!(
            sign_in(&reopened, &mut phone).await.is_ok(),
            "its counter kept"
        );

        let mut elsewhere = Authenticator::new("https://evil.test").unwrap();
        let asked = reopened.get_options().unwrap();
        let answer = elsewhere.get(&asked.options).unwrap();
        assert!(matches!(
            reopened.get(&asked.id, &answer).await,
            Err(BrowserError::Passkey(_))
        ));
    });
}

#[test]
fn a_passkey_s_session_signs_in_from_elsewhere_and_a_link_s_only_here() {
    block_on!(async {
        let clock = ManualClock::at_second(START);
        let browsers = open(store().log(), &clock).await;
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        let (_, by_passkey) = set_up(&browsers, &mut phone).await;
        let link = browsers.link().unwrap();
        let by_link = browsers.redeem(link.expose()).await.unwrap();
        assert!(
            browsers
                .verify(by_passkey.token.expose(), false)
                .await
                .is_ok()
        );
        assert!(matches!(
            browsers.verify(by_link.token.expose(), false).await,
            Err(BrowserError::Elsewhere)
        ));
        assert!(browsers.verify(by_link.token.expose(), true).await.is_ok());
    });
}

#[test]
fn someone_signed_in_adds_a_passkey_with_no_code_and_no_new_session() {
    block_on!(async {
        let clock = ManualClock::at_second(START);
        let browsers = open(store().log(), &clock).await;
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        set_up(&browsers, &mut phone).await;
        let mut laptop = Authenticator::new(ORIGIN).unwrap();
        let asked = browsers.create_options(Proof::SignedIn).unwrap();
        let answer = laptop.create(&asked.options).unwrap();
        assert!(
            matches!(
                browsers.create(&asked.id, &answer, None).await,
                Err(BrowserError::Ceremony)
            ),
            "a request the person made is no setup link's"
        );
        let asked = browsers.create_options(Proof::SignedIn).unwrap();
        let answer = laptop.create(&asked.options).unwrap();
        let added = browsers
            .add(&asked.id, &answer, Some("Laptop"))
            .await
            .unwrap();
        assert_eq!(added.name, "Laptop");
        assert_eq!(browsers.passkeys().len(), 2);
        assert!(sign_in(&browsers, &mut laptop).await.is_ok());
    });
}

#[test]
fn removing_a_passkey_ends_the_sessions_it_began_and_it_signs_in_no_more() {
    block_on!(async {
        let clock = ManualClock::at_second(START);
        let browsers = open(store().log(), &clock).await;
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        let (added, first) = set_up(&browsers, &mut phone).await;
        let second = sign_in(&browsers, &mut phone).await.unwrap();
        let link = browsers
            .redeem(browsers.link().unwrap().expose())
            .await
            .unwrap();
        browsers.remove(&added.id).await.unwrap();
        assert_eq!(browsers.passkeys(), Vec::new());
        for ended in [&first, &second] {
            assert!(browsers.verify(ended.token.expose(), true).await.is_err());
        }
        assert!(
            browsers.verify(link.token.expose(), true).await.is_ok(),
            "a link's stays"
        );
        assert!(matches!(
            sign_in(&browsers, &mut phone).await,
            Err(BrowserError::NoPasskey)
        ));
    });
}

#[test]
fn setup_codes_and_requests_expire_and_codes_that_are_not_one_are_limited() {
    block_on!(async {
        let clock = ManualClock::at_second(START);
        let browsers = open(store().log(), &clock).await;
        let code = browsers.setup().unwrap();
        let asked = browsers.create_options(Proof::Code(code.expose())).unwrap();
        clock.advance_seconds(5 * 60 + 1);
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        let answer = phone.create(&asked.options).unwrap();
        assert!(matches!(
            browsers.create(&asked.id, &answer, None).await,
            Err(BrowserError::Ceremony)
        ));
        clock.advance_seconds(5 * 60);
        assert!(matches!(
            browsers.create_options(Proof::Code(code.expose())),
            Err(BrowserError::Setup)
        ));
        let good = browsers.setup().unwrap();
        // With the expired code above, ten that are not one this minute.
        for _ in 0..9 {
            assert!(browsers.create_options(Proof::Code("guess")).is_err());
        }
        assert!(matches!(
            browsers.create_options(Proof::Code(good.expose())),
            Err(BrowserError::TooMany)
        ));
        clock.advance_seconds(61);
        assert!(browsers.create_options(Proof::Code(good.expose())).is_ok());
    });
}

#[test]
fn a_server_reached_nowhere_makes_no_passkeys_and_only_once_one_exists_signs_in() {
    block_on!(async {
        let clock = ManualClock::at_second(START);
        let here = nowhere(store().log(), &clock).await;
        assert!(matches!(here.setup(), Err(BrowserError::NoPasskeys)));
        assert!(matches!(here.get_options(), Err(BrowserError::NoPasskeys)));
        let reached = open(store().log(), &clock).await;
        assert!(matches!(
            reached.get_options(),
            Err(BrowserError::NoPasskey)
        ));
    });
}

#[test]
fn on_this_computer_passkeys_are_made_at_localhost_once_the_server_listens_there() {
    block_on!(async {
        let clock = ManualClock::at_second(START);
        let here = nowhere(store().log(), &clock).await;
        assert_eq!(here.passkeys_here(), None);
        here.reached_here(4180).unwrap();
        here.reached_here(4181).unwrap();
        assert_eq!(
            here.passkeys_here().as_deref(),
            Some("http://localhost:4180"),
            "the first port it listens at"
        );
        assert!(!here.signs_in_with_passkeys(), "none made yet");
        assert!(matches!(here.get_options(), Err(BrowserError::NoPasskey)));

        // A browser here signed in by glave's link makes one.
        let mut mac = Authenticator::new("http://localhost:4180").unwrap();
        let asked = here.create_options(Proof::SignedIn).unwrap();
        assert_eq!(asked.options["publicKey"]["rp"]["id"], "localhost");
        let answer = mac.create(&asked.options).unwrap();
        here.add(&asked.id, &answer, Some("Mac")).await.unwrap();
        assert!(here.signs_in_with_passkeys());

        // Any browser here signs in with it, its session from this computer alone; a page at
        // another port of this computer, or at its address rather than its name, asking for the
        // same passkey, never does.
        let session = sign_in(&here, &mut mac).await.unwrap();
        assert!(here.verify(session.token.expose(), true).await.is_ok());
        assert!(matches!(
            here.verify(session.token.expose(), false).await,
            Err(BrowserError::Elsewhere)
        ));
        for page in ["http://localhost:5173", "http://127.0.0.1:4180"] {
            mac.on_page(page);
            assert!(
                matches!(
                    sign_in(&here, &mut mac).await,
                    Err(BrowserError::Passkey(_))
                ),
                "{page}"
            );
        }
        mac.on_page("http://localhost:4180");
        assert!(
            sign_in(&here, &mut mac).await.is_ok(),
            "it was the page's origin that was refused"
        );
    });
}

/// The browsers of the server on `log` reached from nowhere else, before it listens here.
async fn nowhere(log: SqliteLog, clock: &ManualClock) -> Browsers {
    Browsers::open(
        log,
        scope(),
        SERVER.into(),
        HANDLE,
        None,
        Box::new(clock.clone()),
    )
    .await
    .unwrap()
}

/// The browsers of the server on `log` reached here alone, at port 4180.
async fn here_on(log: SqliteLog, clock: &ManualClock) -> Browsers {
    let here = nowhere(log, clock).await;
    here.reached_here(4180).unwrap();
    here
}

#[test]
fn a_passkey_signs_in_only_with_the_relying_party_it_was_made_for() {
    block_on!(async {
        let clock = ManualClock::at_second(START);
        // Made on this computer, then the server given a public origin on the same store.
        let kept = store();
        let here = here_on(kept.log(), &clock).await;
        let mut mac = Authenticator::new("http://localhost:4180").unwrap();
        let asked = here.create_options(Proof::SignedIn).unwrap();
        let answer = mac.create(&asked.options).unwrap();
        here.add(&asked.id, &answer, None).await.unwrap();
        let session = sign_in(&here, &mut mac).await.unwrap();
        let reached = open(kept.log(), &clock).await;
        assert!(matches!(
            reached.get_options(),
            Err(BrowserError::NoPasskey)
        ));
        assert!(
            matches!(
                reached.verify(session.token.expose(), false).await,
                Err(BrowserError::Elsewhere)
            ),
            "its session signs in from this computer alone"
        );

        // Made at the public origin, then the server reached here alone on the same store.
        let kept = store();
        let reached = open(kept.log(), &clock).await;
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        set_up(&reached, &mut phone).await;
        let here = here_on(kept.log(), &clock).await;
        assert!(
            !here.signs_in_with_passkeys(),
            "none made for this computer"
        );
        assert!(matches!(here.get_options(), Err(BrowserError::NoPasskey)));
    });
}

#[test]
fn a_server_reached_from_elsewhere_keeps_its_public_origin_as_its_relying_party() {
    block_on!(async {
        let clock = ManualClock::at_second(START);
        let reached = open(store().log(), &clock).await;
        reached.reached_here(4180).unwrap();
        assert_eq!(reached.passkeys_here(), None);
        assert!(
            reached.signs_in_with_passkeys(),
            "a setup link makes the first"
        );
        let code = reached.setup().unwrap();
        let asked = reached.create_options(Proof::Code(code.expose())).unwrap();
        assert_eq!(asked.options["publicKey"]["rp"]["id"], "glave.test");
    });
}

/// `handle` as a passkey keeps it: base64url.
fn encoded(handle: Uuid) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(handle.as_bytes())
}

/// The user handle a request to make a passkey names, as `browsers` begins one.
fn handle_asked(browsers: &Browsers) -> serde_json::Value {
    let code = browsers.setup().unwrap();
    let asked = browsers.create_options(Proof::Code(code.expose())).unwrap();
    asked.options["publicKey"]["user"]["id"].clone()
}

#[test]
fn every_passkey_keeps_the_person_s_handle_through_a_fresh_store_and_each_person_their_own() {
    block_on!(async {
        let clock = ManualClock::at_second(START);
        let first = open(store().log(), &clock).await;
        assert_eq!(handle_asked(&first), encoded(HANDLE));
        // The store started over, the handle kept by the caller: a passkey made again names the
        // same person, so the device's passkey manager replaces the one it made before.
        let fresh = open(store().log(), &clock).await;
        assert_eq!(handle_asked(&fresh), handle_asked(&first));
        // Another person on the same store keeps a handle of their own.
        let other = Uuid::from_u128(0x0dd1_7e5a_5e7a_4a1d_8a2e_0000_0000_0692);
        let theirs = Scope::glaves_own(
            OrgId::from_uuid(Uuid::from_u128(1)),
            ProjectId::from_uuid(Uuid::from_u128(3)),
        );
        let kept = store();
        let mine = open(kept.log(), &clock).await;
        let someone = open_for(kept.log(), theirs, other, &clock).await;
        assert_eq!(handle_asked(&someone), encoded(other));
        assert_eq!(handle_asked(&mine), encoded(HANDLE));
    });
}

#[test]
fn a_sign_in_tells_the_browser_every_passkey_that_signs_the_person_in() {
    block_on!(async {
        let clock = ManualClock::at_second(START);
        let browsers = open(store().log(), &clock).await;
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        let (_, made) = set_up(&browsers, &mut phone).await;
        let only_phone = Accepted {
            party: "glave.test".into(),
            user: encoded(HANDLE),
            credentials: vec![phone.credential()],
        };
        assert_eq!(made.accepted.as_ref(), Some(&only_phone));
        let mut laptop = Authenticator::new(ORIGIN).unwrap();
        let asked = browsers.create_options(Proof::SignedIn).unwrap();
        let answer = laptop.create(&asked.options).unwrap();
        let added = browsers.add(&asked.id, &answer, None).await.unwrap();
        let both = sign_in(&browsers, &mut phone).await.unwrap().accepted;
        let credentials = both.map(|accepted| accepted.credentials);
        assert_eq!(
            credentials,
            Some(vec![phone.credential(), laptop.credential()])
        );
        browsers.remove(&added.id).await.unwrap();
        let after = sign_in(&browsers, &mut phone).await.unwrap();
        assert_eq!(after.accepted, Some(only_phone));
        // A link's page is on the server's computer, not at its public origin.
        let link = browsers.link().unwrap();
        assert_eq!(browsers.redeem(link.expose()).await.unwrap().accepted, None);

        // Here, a link signs in at the relying party's own page: with no passkey made yet, none
        // signs in, so the browser's passkey manager forgets those of a store before.
        let here = here_on(store().log(), &clock).await;
        let link = here.link().unwrap();
        let none = Accepted {
            party: "localhost".into(),
            user: encoded(HANDLE),
            credentials: Vec::new(),
        };
        assert_eq!(
            here.redeem(link.expose()).await.unwrap().accepted,
            Some(none)
        );
        let nowhere = nowhere(store().log(), &clock).await;
        let link = nowhere.link().unwrap();
        assert_eq!(nowhere.redeem(link.expose()).await.unwrap().accepted, None);
    });
}

#[test]
fn a_passkey_the_server_does_not_know_is_named_only_when_it_carries_the_person_s_handle() {
    block_on!(async {
        let clock = ManualClock::at_second(START);
        // Made before the store started over, the person's handle kept.
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        set_up(&open(store().log(), &clock).await, &mut phone).await;
        let kept = store();
        let fresh = open(kept.log(), &clock).await;
        let mut laptop = Authenticator::new(ORIGIN).unwrap();
        let (laptop_s, _) = set_up(&fresh, &mut laptop).await;
        let refused = sign_in(&fresh, &mut phone).await.unwrap_err();
        assert!(
            matches!(&refused, BrowserError::Unknown { party, credential }
                if party == "glave.test" && *credential == phone.credential()),
            "{refused:?}"
        );
        // Another program's passkey for the same relying party is not this server's to judge.
        let other = Uuid::from_u128(0x0dd1_7e5a_5e7a_4a1d_8a2e_0000_0000_0693);
        let mut stranger = Authenticator::new(ORIGIN).unwrap();
        let elsewhere = open_for(store().log(), scope(), other, &clock).await;
        set_up(&elsewhere, &mut stranger).await;
        assert!(matches!(
            sign_in(&fresh, &mut stranger).await,
            Err(BrowserError::Passkey(_))
        ));
        // Removed on another server on this store after the request began.
        let asked = fresh.get_options().unwrap();
        let another = open(kept.log(), &clock).await;
        another.remove(&laptop_s.id).await.unwrap();
        let answer = laptop.get(&asked.options).unwrap();
        assert!(matches!(
            fresh.get(&asked.id, &answer).await,
            Err(BrowserError::Unknown { credential, .. }) if credential == laptop.credential()
        ));
    });
}
