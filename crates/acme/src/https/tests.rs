use glave_ports::testing::{CountingEntropy, ManualClock};
use glave_ports::{OrgId, ProjectId, block_on};
use glave_store_sqlite::{SqliteLog, Store};
use uuid::Uuid;

use super::*;

const NAME: &str = "glave.test";
const DIRECTORY: &str = "https://acme.example.com/directory";

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

fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

async fn open(log: SqliteLog) -> Https<SqliteLog> {
    let clock = ManualClock::at_second(at("2026-10-20T09:00:00Z").as_second());
    let entropy = CountingEntropy::starting_at(1);
    Https::open(
        log,
        scope(),
        NAME,
        DIRECTORY,
        Box::new(clock),
        Box::new(entropy),
    )
    .await
    .unwrap()
}

/// A certificate for `name` valid from `began` to `ends`, and its key, as PEM.
fn issued(name: &str, began: &str, ends: &str) -> CertificateIssued {
    let key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::new(vec![name.to_owned()]).unwrap();
    let seconds = |text: &str| {
        rcgen::date_time_ymd(1970, 1, 1) + Duration::from_secs(at(text).as_second().unsigned_abs())
    };
    params.not_before = seconds(began);
    params.not_after = seconds(ends);
    let certificate = params.self_signed(&key).unwrap();
    CertificateIssued {
        name: name.to_owned(),
        chain: certificate.pem(),
        key: Secret::new(key.serialize_pem()),
        not_before: At::from(at(began)),
        not_after: At::from(at(ends)),
    }
}

#[test]
fn a_certificate_is_due_with_a_third_of_its_life_left() {
    let began = at("2026-10-01T00:00:00Z");
    let ends = at("2026-12-30T00:00:00Z");
    assert_eq!(due(began, ends), at("2026-11-30T00:00:00Z"));
    let now = at("2026-11-29T23:00:00Z");
    assert_eq!(until(Some(due(began, ends)), now), Duration::from_hours(1));
    assert_eq!(
        until(Some(due(began, ends)), at("2026-10-02T00:00:00Z")),
        LOOK_AGAIN,
        "a long wait is looked at again daily"
    );
    assert_eq!(until(Some(due(began, ends)), ends), Duration::ZERO);
    assert_eq!(until(None, now), Duration::ZERO, "none in hand: at once");
}

#[test]
fn failed_orders_wait_longer_each_time_up_to_six_hours() {
    let waits: Vec<u64> = (1..=8).map(|n| backoff(n).as_secs() / 60).collect();
    assert_eq!(waits, [5, 10, 20, 40, 80, 160, 320, 360]);
    assert_eq!(backoff(u32::MAX), LONGEST_WAIT);
}

#[test]
fn a_certificate_s_life_is_read_from_it() {
    let issued = issued(NAME, "2026-10-01T00:00:00Z", "2026-12-30T00:00:00Z");
    let (began, ends) = life(&issued.chain).unwrap();
    assert_eq!(
        (began, ends),
        (at("2026-10-01T00:00:00Z"), at("2026-12-30T00:00:00Z"))
    );
    assert!(life("not a certificate").is_err());
    assert!(life("").is_err());
}

#[test]
fn a_certificate_kept_for_the_name_is_served_at_once_and_another_name_s_is_not() {
    block_on!(async {
        let store = store();
        let fresh = open(store.log()).await;
        assert!(!fresh.ready(), "nothing kept: nothing to serve");
        assert_eq!(fresh.due(), None);
        let ledger = Ledger {
            log: store.log(),
            scope: scope(),
            clock: Box::new(ManualClock::at_second(0)),
            entropy: Box::new(CountingEntropy::starting_at(9)),
        };
        let other = issued("other.test", "2026-10-01T00:00:00Z", "2026-12-30T00:00:00Z");
        ledger.issued(&other).await.unwrap();
        assert!(
            !open(store.log()).await.ready(),
            "another name's is not this one's"
        );
        let ours = issued(NAME, "2026-10-01T00:00:00Z", "2026-12-30T00:00:00Z");
        ledger.issued(&ours).await.unwrap();
        let reopened = open(store.log()).await;
        assert!(reopened.ready());
        assert_eq!(reopened.due(), Some(at("2026-11-30T00:00:00Z")));
    });
}

#[test]
fn an_order_the_authority_never_answers_fails_at_its_limit() {
    // Listening, never answering: the system takes the connection and nothing reads it.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let directory = format!("https://{}/directory", listener.local_addr().unwrap());
    let renewed = block_on!(async {
        let clock = ManualClock::at_second(at("2026-10-20T09:00:00Z").as_second());
        let https = Https::open(
            store().log(),
            scope(),
            NAME,
            &directory,
            Box::new(clock),
            Box::new(CountingEntropy::starting_at(1)),
        )
        .await
        .unwrap();
        // The clock jumps to the limit as soon as nothing else can run.
        tokio::time::pause();
        https.renew().await
    });
    let Err(error) = renewed else {
        panic!("an order no one answered was issued");
    };
    assert_eq!(
        error.to_string(),
        "the certificate authority did not finish the order within 10 minutes"
    );
}

#[test]
fn an_account_is_kept_for_its_directory_alone() {
    block_on!(async {
        let store = store();
        let ledger = Ledger {
            log: store.log(),
            scope: scope(),
            clock: Box::new(ManualClock::at_second(0)),
            entropy: Box::new(CountingEntropy::starting_at(1)),
        };
        let account = |directory: &str| AcmeRegistered {
            directory: directory.to_owned(),
            credentials: Secret::new(format!("{{\"for\":\"{directory}\"}}")),
        };
        ledger.registered(&account(DIRECTORY)).await.unwrap();
        ledger
            .registered(&account("https://other.example.com/directory"))
            .await
            .unwrap();
        let kept = ledger.read(NAME, DIRECTORY).await.unwrap();
        assert_eq!(kept.account, Some(account(DIRECTORY)));
        assert_eq!(kept.certificate, None);
    });
}
