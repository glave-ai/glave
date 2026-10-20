use std::sync::Arc;

use glave_ports::testing::ManualClock;
use glave_ports::{OrgId, ProjectId, Scope, block_on};
use glave_store_sqlite::{SqliteLog, Store};

use super::*;
use crate::testing::Authenticator;

const START: i64 = 1_791_590_400;
const ORIGIN: &str = "https://glave.test";
const SERVER: &str = "0192f5c6-0000-7000-8000-0000000005e7";

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
    let origin = PublicOrigin::parse(ORIGIN).unwrap();
    Browsers::open(
        log,
        scope(),
        SERVER.into(),
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
            browsers.verify(session.token.expose()).await.unwrap(),
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
        assert!(browsers.verify(session.token.expose()).await.is_ok());

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
            assert!(browsers.verify(ended.token.expose()).await.is_err());
        }
        assert!(
            browsers.verify(link.token.expose()).await.is_ok(),
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
fn only_a_server_reached_from_elsewhere_makes_passkeys_and_only_once_one_exists_signs_in() {
    block_on!(async {
        let clock = ManualClock::at_second(START);
        let here = Browsers::open(
            store().log(),
            scope(),
            SERVER.into(),
            None,
            Box::new(clock.clone()),
        )
        .await
        .unwrap();
        assert!(matches!(here.setup(), Err(BrowserError::NoPasskeys)));
        assert!(matches!(here.get_options(), Err(BrowserError::NoPasskeys)));
        let reached = open(store().log(), &clock).await;
        assert!(matches!(
            reached.get_options(),
            Err(BrowserError::NoPasskey)
        ));
    });
}
