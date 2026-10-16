//! The accounts ledger, on a store in memory: renewals kept only while what they renew is.

use glave_event::{CredentialConnected, CredentialId, Forge, Principal, Secret};
use glave_ports::EventLog as _;
use jiff::Timestamp;

use super::{Ledger, random};

fn id() -> CredentialId {
    CredentialId::new(Timestamp::now(), random().unwrap()).unwrap()
}

fn host(token: &str) -> CredentialConnected {
    CredentialConnected {
        credential: id(),
        forge: Forge::Github,
        url: "https://github.com".into(),
        username: "octocat".into(),
        token: Secret::new(token),
        login: Some("octocat".into()),
        scopes: None,
        renewal: None,
        by: Principal::glave(),
    }
}

/// `old` renewed: a new id and token.
fn renewal(old: &CredentialConnected, token: &str) -> CredentialConnected {
    CredentialConnected {
        credential: id(),
        token: Secret::new(token),
        ..old.clone()
    }
}

async fn ledger() -> Ledger {
    Ledger::in_memory().await.unwrap()
}

async fn kept(ledger: &Ledger) -> Vec<CredentialId> {
    let accounts = ledger.accounts().await.unwrap();
    accounts
        .credentials
        .iter()
        .map(|kept| kept.credential)
        .collect()
}

fn block_on<T>(future: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

#[test]
fn a_renewal_takes_its_place_and_the_old_id_still_names_it() {
    block_on(async {
        let ledger = ledger().await;
        let first = host("ghu_first");
        ledger.keep_credential(&first).await.unwrap();
        let second = renewal(&first, "ghu_second");
        assert!(
            ledger
                .renew_credential(first.credential, &second)
                .await
                .unwrap()
        );
        let third = renewal(&second, "ghu_third");
        assert!(
            ledger
                .renew_credential(second.credential, &third)
                .await
                .unwrap()
        );
        assert_eq!(kept(&ledger).await, [third.credential]);
        let accounts = ledger.accounts().await.unwrap();
        let named = accounts.credential(first.credential).unwrap();
        assert_eq!(named.token.expose(), "ghu_third");
        // The renewed tokens' keys are destroyed: their events read as erased.
        let ledger_id = super::ledger().unwrap();
        let raw = ledger.log.read(&ledger.scope, ledger_id, None, 100).await;
        let data: Vec<String> = raw
            .unwrap()
            .iter()
            .map(|e| e.data.get().to_owned())
            .collect();
        let holding = |token: &str| data.iter().filter(|data| data.contains(token)).count();
        assert_eq!(
            (holding("ghu_first"), holding("ghu_second")),
            (0, 0),
            "{data:?}"
        );
        assert_eq!(holding("ghu_third"), 1);

        assert!(ledger.remove_credential(first.credential).await.unwrap());
        assert_eq!(kept(&ledger).await, []);
        assert!(!ledger.remove_credential(third.credential).await.unwrap());
    });
}

#[test]
fn a_host_removed_or_replaced_while_renewed_stays_so() {
    block_on(async {
        let ledger = ledger().await;
        let signed_in = host("ghu_signed_in");
        ledger.keep_credential(&signed_in).await.unwrap();
        assert!(
            ledger
                .remove_credential(signed_in.credential)
                .await
                .unwrap()
        );
        let late = renewal(&signed_in, "ghu_late");
        assert!(
            !ledger
                .renew_credential(signed_in.credential, &late)
                .await
                .unwrap()
        );
        assert_eq!(kept(&ledger).await, []);

        let again = host("ghu_again");
        ledger.keep_credential(&again).await.unwrap();
        let pasted = host("github_pat_pasted");
        ledger.keep_credential(&pasted).await.unwrap();
        let late = renewal(&again, "ghu_late_again");
        assert!(
            !ledger
                .renew_credential(again.credential, &late)
                .await
                .unwrap()
        );
        assert_eq!(kept(&ledger).await, [pasted.credential]);
        assert!(
            !ledger.remove_credential(again.credential).await.unwrap(),
            "a host replaced by the person's own names nothing"
        );
    });
}
