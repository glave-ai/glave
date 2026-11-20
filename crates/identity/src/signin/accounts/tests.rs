use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use glave_ports::testing::ManualClock;
use glave_ports::{OrgId, block_on};
use glave_store_sqlite::Store;
use serde_json::json;
use uuid::Uuid;

use super::*;
use crate::testing::Authenticator;

/// What `accounts` refuses `forged` with, a passkey no account here knows, once it says it is
/// `handle`'s, as anyone's answer can.
async fn refused(
    accounts: &Accounts,
    forged: &mut Authenticator,
    party: &str,
    handle: Uuid,
) -> String {
    let user = URL_SAFE_NO_PAD.encode(handle.as_bytes());
    let said = json!({"publicKey": {"user": {"id": user}, "rp": {"id": party}}});
    forged.create(&said).unwrap();
    let asked = accounts.get_options().unwrap();
    let answer = forged.get(&asked.options).unwrap();
    let discovered = accounts.identify(&asked.id, &answer).unwrap();
    let refused = accounts.signed_in_by(discovered).await.err();
    format!("{refused:?}")
}

#[test]
fn a_passkey_no_account_knows_is_refused_alike_whoever_s_id_it_names() {
    block_on!(async {
        let origins = [
            ("https://glave.test", "glave.test", true),
            ("http://localhost:4180", "localhost", false),
        ];
        for (origin, party, forgotten) in origins {
            let root = glave_keys::InMemoryRoot::new().unwrap();
            let store =
                Store::open_in_memory(glave_keys::KeySettings::new(Arc::new(root))).unwrap();
            let origin = PublicOrigin::parse(origin).unwrap();
            let clock = Arc::new(ManualClock::at_second(1_791_590_400));
            let directory = Arc::new(store.directory());
            let accounts =
                Accounts::new(store.log(), directory, "server".into(), &origin, clock).unwrap();
            let (member, _) = accounts.made().await.unwrap();
            let org = OrgId::from_uuid(Uuid::from_u128(1));
            accounts.join(member, org).await.unwrap();
            let never = Uuid::from_u128(0x0192_f5c6_0000_7000_8000_0000_0000_0bad);

            let mut forged = Authenticator::new(origin.as_str()).unwrap();
            let theirs = refused(&accounts, &mut forged, party, member.as_uuid()).await;
            let no_one_s = refused(&accounts, &mut forged, party, never).await;
            assert_eq!(theirs, no_one_s, "the same answer either way at {party}");
            // Forgotten by the passkey manager only where the server's name is its own.
            assert_eq!(theirs.contains("Unknown"), forgotten, "{theirs}");
        }
    });
}

/// A made-up answer naming a member's id is refused before that member's ledger is opened, as
/// one naming no one's is, so how long it takes says nothing of whether the id has an account
/// (#1523): its credential is looked up first, among every account's passkeys.
#[test]
fn a_passkey_no_account_has_is_refused_before_any_ledger_is_opened() {
    block_on!(async {
        let root = glave_keys::InMemoryRoot::new().unwrap();
        let store = Store::open_in_memory(glave_keys::KeySettings::new(Arc::new(root))).unwrap();
        let origin = PublicOrigin::parse("https://glave.test").unwrap();
        let clock = Arc::new(ManualClock::at_second(1_791_590_400));
        let open = || {
            let directory = Arc::new(store.directory());
            let clock = Arc::<ManualClock>::clone(&clock);
            Accounts::new(store.log(), directory, "server".into(), &origin, clock).unwrap()
        };
        let before = open();
        let (member, _) = before.made().await.unwrap();
        before
            .join(member, OrgId::from_uuid(Uuid::from_u128(1)))
            .await
            .unwrap();
        // Another server on the store, which has opened no one's ledger yet.
        let accounts = open();
        let mut forged = Authenticator::new(origin.as_str()).unwrap();
        refused(&accounts, &mut forged, "glave.test", member.as_uuid()).await;
        assert!(
            !accounts.held().contains_key(&member),
            "the member's ledger was opened"
        );
    });
}
