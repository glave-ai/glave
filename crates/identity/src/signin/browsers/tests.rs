use std::sync::Arc;

use glave_ports::testing::ManualClock;
use glave_ports::{OrgId, ProjectId, block_on};
use glave_store_sqlite::{SqliteLog, Store};

use super::*;

const START: i64 = 1_791_590_400;

/// A store of its own, in memory.
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

async fn open(log: SqliteLog, clock: &ManualClock) -> Browsers {
    Browsers::open(log, scope(), "server".into(), Box::new(clock.clone()))
        .await
        .unwrap()
}

fn later(clock: &ManualClock, by: SignedDuration) {
    clock.set_millis(clock.millis() + i64::try_from(by.as_millis()).unwrap());
}

/// Every event the ledger holds, as written.
async fn written(log: &SqliteLog) -> String {
    let events = log
        .read(&scope(), Reserved::Browsers.id(), None, 1_000)
        .await
        .unwrap();
    events
        .iter()
        .map(|event| String::from_utf8_lossy(event.data.get().as_bytes()).into_owned())
        .collect()
}

#[test]
fn a_code_works_once_and_expires() {
    block_on!(async {
        let clock = ManualClock::at_second(START);
        let browsers = open(store().log(), &clock).await;
        let code = browsers.link().unwrap();
        let session = browsers.redeem(code.expose()).await.unwrap();
        assert!(session.token.expose().starts_with(SESSION_PREFIX));
        assert!(matches!(
            browsers.redeem(code.expose()).await,
            Err(BrowserError::Code)
        ));

        let code = browsers.link().unwrap();
        later(&clock, CODE_LIFETIME + SignedDuration::from_secs(1));
        assert!(matches!(
            browsers.redeem(code.expose()).await,
            Err(BrowserError::Code)
        ));
        assert!(matches!(
            browsers.redeem("not a code").await,
            Err(BrowserError::Code)
        ));
    });
}

#[test]
fn a_session_survives_a_restart_and_the_store_keeps_only_hashes() {
    block_on!(async {
        let clock = ManualClock::at_second(START);
        let store = store();
        let browsers = open(store.log(), &clock).await;
        let code = browsers.link().unwrap();
        let session = browsers.redeem(code.expose()).await.unwrap();
        drop(browsers);

        let kept = written(&store.log()).await;
        assert!(kept.contains("hash"), "{kept}");
        assert!(!kept.contains(session.token.expose()), "no token kept");
        assert!(!kept.contains(code.expose()), "no code kept");
        let again = open(store.log(), &clock).await;
        assert_eq!(
            again.verify(session.token.expose()).await.unwrap(),
            session.id
        );
        assert!(matches!(
            again.verify(&format!("{SESSION_PREFIX}00")).await,
            Err(BrowserError::Ended)
        ));
    });
}

#[test]
fn a_browser_signed_in_on_one_server_is_signed_in_on_another_on_the_same_store() {
    block_on!(async {
        let clock = ManualClock::at_second(START);
        let store = store();
        let first = open(store.log(), &clock).await;
        let second = open(store.log(), &clock).await;
        let session = first.redeem(first.link().unwrap().expose()).await.unwrap();
        assert_eq!(
            second.verify(session.token.expose()).await.unwrap(),
            session.id
        );
        // Both write; neither loses the other's.
        let other = second
            .redeem(second.link().unwrap().expose())
            .await
            .unwrap();
        first.end(&session.id).await.unwrap();
        assert!(
            second.verify(session.token.expose()).await.is_ok(),
            "until it looks again"
        );
        second.end_others(Some(&other.id)).await.unwrap();
        assert_eq!(first.verify(other.token.expose()).await.unwrap(), other.id);
        assert!(first.verify(session.token.expose()).await.is_err());
    });
}

#[test]
fn a_session_lasts_thirty_days_from_its_last_use() {
    block_on!(async {
        let clock = ManualClock::at_second(START);
        let store = store();
        let browsers = open(store.log(), &clock).await;
        let used = browsers
            .redeem(browsers.link().unwrap().expose())
            .await
            .unwrap();
        let idle = browsers
            .redeem(browsers.link().unwrap().expose())
            .await
            .unwrap();
        later(&clock, SESSION_LIFETIME - SignedDuration::from_hours(1));
        browsers.verify(used.token.expose()).await.unwrap();
        later(&clock, SignedDuration::from_hours(2));
        // Renewed when it was used, and the renewal outlives a restart.
        let browsers = open(store.log(), &clock).await;
        browsers.verify(used.token.expose()).await.unwrap();
        assert!(matches!(
            browsers.verify(idle.token.expose()).await,
            Err(BrowserError::Ended)
        ));
    });
}

#[test]
fn signing_out_ends_a_session_and_other_browsers_end_theirs() {
    block_on!(async {
        let clock = ManualClock::at_second(START);
        let browsers = open(store().log(), &clock).await;
        let mut sessions = Vec::new();
        for _ in 0..3 {
            let code = browsers.link().unwrap();
            sessions.push(browsers.redeem(code.expose()).await.unwrap());
        }
        let [this, other, gone] = sessions.as_slice() else {
            panic!("three sessions");
        };
        browsers.end(&gone.id).await.unwrap();
        assert!(browsers.verify(gone.token.expose()).await.is_err());
        browsers.end_others(Some(&this.id)).await.unwrap();
        assert!(browsers.verify(other.token.expose()).await.is_err());
        assert_eq!(browsers.verify(this.token.expose()).await.unwrap(), this.id);
        browsers.end_others(None).await.unwrap();
        assert!(browsers.verify(this.token.expose()).await.is_err());
    });
}

#[test]
fn codes_and_sessions_are_bounded() {
    block_on!(async {
        let clock = ManualClock::at_second(START);
        let browsers = open(store().log(), &clock).await;
        let first = browsers.link().unwrap();
        for _ in 0..MOST_CODES {
            browsers.link().unwrap();
        }
        assert!(browsers.redeem(first.expose()).await.is_err(), "forgotten");
        let oldest = browsers
            .redeem(browsers.link().unwrap().expose())
            .await
            .unwrap();
        for _ in 0..MOST_SESSIONS {
            later(&clock, SignedDuration::from_secs(1));
            let code = browsers.link().unwrap();
            browsers.redeem(code.expose()).await.unwrap();
        }
        assert!(browsers.verify(oldest.token.expose()).await.is_err());
    });
}
