use glave_ports::block_on;
use glave_ports::testing::ManualClock;

use super::*;

const START: i64 = 1_791_590_400;

fn place(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("glave-browsers-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("browsers.json")
}

fn open(path: &std::path::Path, clock: &ManualClock) -> Browsers {
    Browsers::open(path.to_owned(), Box::new(clock.clone())).unwrap()
}

fn later(clock: &ManualClock, by: SignedDuration) {
    clock.set_millis(clock.millis() + i64::try_from(by.as_millis()).unwrap());
}

#[test]
fn a_code_works_once_and_expires() {
    block_on!(async {
        let clock = ManualClock::at_second(START);
        let browsers = open(&place("code"), &clock);
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
fn a_session_survives_a_restart_and_keeps_only_hashes() {
    block_on!(async {
        let clock = ManualClock::at_second(START);
        let path = place("restart");
        let browsers = open(&path, &clock);
        let server = browsers.server();
        let code = browsers.link().unwrap();
        let session = browsers.redeem(code.expose()).await.unwrap();
        drop(browsers);

        let kept = std::fs::read_to_string(&path).unwrap();
        assert!(!kept.contains(session.token.expose()), "no token on disk");
        assert!(!kept.contains(code.expose()), "no code on disk");
        let again = open(&path, &clock);
        assert_eq!(again.server(), server, "the same server after a restart");
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
fn a_session_lasts_thirty_days_from_its_last_use() {
    block_on!(async {
        let clock = ManualClock::at_second(START);
        let path = place("lifetime");
        let browsers = open(&path, &clock);
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
        let browsers = open(&path, &clock);
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
        let browsers = open(&place("out"), &clock);
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
fn an_unreadable_file_starts_over_as_another_server() {
    let clock = ManualClock::at_second(START);
    let path = place("unreadable");
    let first = open(&path, &clock).server();
    std::fs::write(&path, "not json").unwrap();
    assert_ne!(open(&path, &clock).server(), first);
}

#[test]
fn codes_and_sessions_are_bounded() {
    block_on!(async {
        let clock = ManualClock::at_second(START);
        let browsers = open(&place("bounded"), &clock);
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
