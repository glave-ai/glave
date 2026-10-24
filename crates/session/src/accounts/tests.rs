//! The accounts ledger, on a sealed store in memory: renewals kept only while what they renew
//! is, and a removed account erased.

use std::sync::Arc;

use glave_event::{
    ConnectionConnected, CredentialConnected, CredentialId, Forge, Principal, PrincipalId,
    ProviderConnected, ProviderId, ProviderKind, Secret, SignInClient,
};
use glave_keys::{InMemoryRoot, KeySettings};
use glave_ports::stream::Reserved;
use glave_ports::testing::{CountingEntropy, ManualClock, fake_secret};
use glave_ports::{EventLog as _, OrgId, ProjectId, Scope, block_on};
use glave_store_sqlite::{SqliteLog, Store};

use super::AccountLedger;

/// 2026-10-03, in milliseconds.
const START: i64 = 1_791_000_000_000;

fn uuid(n: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(0x0192_f5c0_0000_7000_8000_0000_0000_0000 | n)
}

struct Kit {
    store: Store,
    ledger: AccountLedger<SqliteLog>,
}

fn kit() -> Kit {
    let keys = KeySettings::new(Arc::new(InMemoryRoot::new().unwrap()));
    let store = Store::open_in_memory(keys).unwrap();
    let scope = Scope::glaves_own(OrgId::from_uuid(uuid(1)), ProjectId::from_uuid(uuid(100)));
    let ledger = AccountLedger::new(
        store.log(),
        store.keyring(),
        scope,
        PrincipalId::try_from(uuid(50)).unwrap(),
        Arc::new(ManualClock::ticking(START)),
        Arc::new(CountingEntropy::starting_at(1)),
    );
    Kit { store, ledger }
}

fn credential(n: u128) -> CredentialId {
    CredentialId::try_from(uuid(1_000 + n)).unwrap()
}

fn host(n: u128, token: &str) -> CredentialConnected {
    CredentialConnected {
        credential: credential(n),
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

/// `old` renewed as the `n`th: a new id and token.
fn renewal(old: &CredentialConnected, n: u128, token: &str) -> CredentialConnected {
    CredentialConnected {
        credential: credential(n),
        token: Secret::new(token),
        ..old.clone()
    }
}

async fn kept(ledger: &AccountLedger<SqliteLog>) -> Vec<CredentialId> {
    let accounts = ledger.accounts().await.unwrap();
    accounts
        .credentials
        .iter()
        .map(|kept| kept.credential)
        .collect()
}

/// The data of every event in the ledger, as the store gives it back.
async fn stored(kit: &Kit) -> Vec<String> {
    let ledger = &kit.ledger;
    let raw = ledger
        .log
        .read(&ledger.scope, Reserved::Accounts.id(), None, 100);
    raw.await
        .unwrap()
        .iter()
        .map(|event| event.data.get().to_owned())
        .collect()
}

#[test]
fn a_renewal_takes_its_place_and_the_old_id_still_names_it() {
    block_on!(async {
        let kit = kit();
        let ledger = &kit.ledger;
        let tokens = [
            fake_secret("ghu_"),
            fake_secret("ghu_"),
            fake_secret("ghu_"),
        ];
        let [one, two, three] = &tokens;
        let first = host(1, one);
        ledger.keep_credential(&first).await.unwrap();
        let second = renewal(&first, 2, two);
        let renewed = ledger.renew_credential(first.credential, &second).await;
        assert!(renewed.unwrap());
        let third = renewal(&second, 3, three);
        let renewed = ledger.renew_credential(second.credential, &third).await;
        assert!(renewed.unwrap());
        assert_eq!(kept(ledger).await, [third.credential]);
        let accounts = ledger.accounts().await.unwrap();
        let named = accounts.credential(first.credential).unwrap();
        assert_eq!(named.token.expose(), three);
        // The renewed tokens' keys are destroyed: their events read as erased.
        let data = stored(&kit).await;
        let holding = |token: &str| data.iter().filter(|data| data.contains(token)).count();
        assert_eq!((holding(one), holding(two)), (0, 0), "{data:?}");
        assert_eq!(holding(three), 1);

        assert!(ledger.remove_credential(first.credential).await.unwrap());
        assert_eq!(kept(ledger).await, []);
        assert!(!ledger.remove_credential(third.credential).await.unwrap());
    });
}

#[test]
fn a_host_removed_or_replaced_while_renewed_stays_so() {
    block_on!(async {
        let kit = kit();
        let ledger = &kit.ledger;
        let signed_in = host(1, &fake_secret("ghu_"));
        ledger.keep_credential(&signed_in).await.unwrap();
        assert!(
            ledger
                .remove_credential(signed_in.credential)
                .await
                .unwrap()
        );
        let late = renewal(&signed_in, 2, &fake_secret("ghu_"));
        let renewed = ledger.renew_credential(signed_in.credential, &late).await;
        assert!(!renewed.unwrap());
        assert_eq!(kept(ledger).await, []);

        let again = host(3, &fake_secret("ghu_"));
        ledger.keep_credential(&again).await.unwrap();
        let pasted = host(4, &fake_secret("github_pat_"));
        ledger.keep_credential(&pasted).await.unwrap();
        let late = renewal(&again, 5, &fake_secret("ghu_"));
        let renewed = ledger.renew_credential(again.credential, &late).await;
        assert!(!renewed.unwrap());
        assert_eq!(kept(ledger).await, [pasted.credential]);
        assert!(
            !ledger.remove_credential(again.credential).await.unwrap(),
            "a host replaced by the person's own names nothing"
        );
    });
}

#[test]
fn erasing_an_account_destroys_its_key_and_it_reads_as_erased() {
    block_on!(async {
        let kit = kit();
        let ledger = &kit.ledger;
        let key = fake_secret("sk-");
        let account = ProviderConnected {
            provider: ProviderId::try_from(uuid(2_000)).unwrap(),
            kind: ProviderKind::Openai,
            url: None,
            key: Secret::new(&key),
            models: vec!["gpt".into()],
            by: Principal::glave(),
        };
        ledger.keep_provider(&account).await.unwrap();
        assert!(stored(&kit).await[0].contains(&key));

        assert!(ledger.remove_provider(account.provider).await.unwrap());
        assert_eq!(ledger.accounts().await.unwrap().providers, []);
        let data = stored(&kit).await;
        assert_eq!(data[0], "{}", "the account reads as erased: {data:?}");
        let keys = kit.store.keys();
        assert!(
            !keys.rotation_due(&ledger.scope).await.unwrap(),
            "the project key was rotated, so backups lose it too"
        );
        assert!(!ledger.remove_provider(account.provider).await.unwrap());
    });
}

/// The connection at `url`, the `n`th signed in, with `token`.
fn connection(n: u128, url: &str, token: &str) -> ConnectionConnected {
    ConnectionConnected {
        connection: credential(n),
        url: url.into(),
        token: Secret::new(token),
        expires: None,
        refresh: Some(Secret::new(fake_secret("mcr_"))),
        client: SignInClient {
            issuer: None,
            client_id: "client".into(),
            scopes: Vec::new(),
        },
        by: Principal::glave(),
    }
}

/// How many of `data` hold `token`.
fn holding(data: &[String], token: &str) -> usize {
    data.iter().filter(|data| data.contains(token)).count()
}

#[test]
fn a_connection_is_kept_by_its_address_renewed_while_kept_and_erased_when_removed() {
    block_on!(async {
        let kit = kit();
        let ledger = &kit.ledger;
        let url = "https://mcp.example.com/mcp";
        let [one, two, three] = [(); 3].map(|()| fake_secret("mcp_"));
        let first = connection(1, url, &one);
        ledger.keep_connection(&first).await.unwrap();
        let other = connection(2, "https://docs.example.com/mcp", &three);
        ledger.keep_connection(&other).await.unwrap();
        let renewed = connection(3, url, &two);
        let kept = ledger.renew_connection(first.connection, &renewed).await;
        assert!(kept.unwrap());
        let again = ledger.renew_connection(first.connection, &renewed).await;
        assert!(
            !again.unwrap(),
            "a connection renewed already is renewed no more"
        );
        let accounts = ledger.accounts().await.unwrap();
        assert_eq!(accounts.connections, [other.clone(), renewed]);
        let data = stored(&kit).await;
        assert_eq!((holding(&data, &one), holding(&data, &two)), (0, 1));

        assert!(ledger.remove_connection(url).await.unwrap());
        assert!(!ledger.remove_connection(url).await.unwrap());
        assert_eq!(ledger.accounts().await.unwrap().connections, [other]);
        let data = stored(&kit).await;
        assert_eq!(holding(&data, &two), 0, "its key is destroyed: {data:?}");
    });
}

#[test]
fn a_ledger_read_by_two_processes_shows_each_what_the_other_changed() {
    block_on!(async {
        let kit = kit();
        let elsewhere = AccountLedger::new(
            kit.store.log(),
            kit.store.keyring(),
            kit.ledger.scope,
            kit.ledger.person,
            Arc::new(ManualClock::ticking(START)),
            Arc::new(CountingEntropy::starting_at(100)),
        );
        let url = "https://mcp.example.com/mcp";
        assert_eq!(elsewhere.accounts().await.unwrap().connections, []);
        let token = fake_secret("mcp_");
        kit.ledger
            .keep_connection(&connection(1, url, &token))
            .await
            .unwrap();
        let seen = elsewhere.accounts().await.unwrap().connections;
        assert_eq!(seen.len(), 1, "what the other kept, at once");
        assert!(elsewhere.remove_connection(url).await.unwrap());
        assert_eq!(kit.ledger.accounts().await.unwrap().connections, []);
        // Each erases what it saw removed; the second finds its key gone already.
        assert_eq!(elsewhere.accounts().await.unwrap().connections, []);
        let data = stored(&kit).await;
        assert_eq!(holding(&data, &token), 0, "erased: {data:?}");
    });
}

#[test]
fn one_person_s_connections_are_never_another_s() {
    block_on!(async {
        let kit = kit();
        let theirs = Scope::glaves_own(OrgId::from_uuid(uuid(1)), ProjectId::from_uuid(uuid(101)));
        let other = AccountLedger::new(
            kit.store.log(),
            kit.store.keyring(),
            theirs,
            PrincipalId::try_from(uuid(51)).unwrap(),
            Arc::new(ManualClock::ticking(START)),
            Arc::new(CountingEntropy::starting_at(100)),
        );
        let url = "https://mcp.example.com/mcp";
        kit.ledger
            .keep_connection(&connection(1, url, &fake_secret("mcp_")))
            .await
            .unwrap();
        assert_eq!(other.accounts().await.unwrap().connections, []);
        assert!(!other.remove_connection(url).await.unwrap());
        let mine = kit.ledger.accounts().await.unwrap().connections;
        assert_eq!(mine.len(), 1, "removing theirs left mine");
    });
}
