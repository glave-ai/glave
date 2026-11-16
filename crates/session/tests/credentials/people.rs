//! Git hosts where people have computers of their own: two people signed in with glave
//! accounts in one organization each connect their own GitHub, list only their own, can't
//! remove the other's, and their threads on their own computers are signed in with their own
//! alone. Signed in to a server whose people have none, they connect nothing.

use std::sync::Arc;

use glave_event::{Auth, AuthMethod, PrincipalId};
use glave_harness::Homes;
use glave_ports::Caller;
use glave_ports::testing::any_secret;
use glave_session::Answering;
use glave_session::protocol::{Request, code};
use serde_json::Value;

use super::{People, connect};
use crate::homes::Here;
use crate::owners_sign_ins::signed_in_for;
use crate::support::*;

/// Someone signed in with a glave account, `n` setting them apart.
fn account(n: u8) -> Caller {
    Caller {
        person: Some(PrincipalId::new(jiff::Timestamp::UNIX_EPOCH, [n; 10]).unwrap()),
        auth: Some(Auth {
            method: AuthMethod::Account,
            issuer: None,
            subject: None,
            session: None,
        }),
        ..Caller::default()
    }
}

/// `caller`'s git hosts, as `credential/list` answers them.
async fn listed(server: &Tested, caller: &Caller) -> Vec<Value> {
    let listed = server
        .request_as(caller, Request::CredentialList {})
        .await
        .unwrap();
    let listed = serde_json::to_value(listed).unwrap();
    listed["credentials"].as_array().unwrap().clone()
}

#[test]
fn people_with_computers_of_their_own_each_connect_their_own_git_hosts_and_use_only_theirs() {
    let dir = folder("hosts-people");
    block_on!(async {
        let answers = vec![answer(Some("Hi."), None), answer(Some("Hi."), None)];
        let mut setup = Setup::asking(answers, Answering::NoOne);
        setup.hosts = Some(Arc::new(People::default()));
        setup.homes = Some(Arc::new(Here::new(&dir.join("people"))) as Arc<dyn Homes>);
        let (server, seen) = setup.build(&dir).await;
        let (ada, grace) = (account(1), account(2));

        let told =
            serde_json::to_value(server.request_as(&ada, Request::Discover {}).await.unwrap())
                .unwrap();
        assert_eq!(told["connects"]["git_hosts"], true, "{told}");
        let token = any_secret();
        let connected = server
            .request_as(&ada, connect("https://github.com", &token))
            .await
            .unwrap();
        let connected = serde_json::to_value(connected).unwrap();
        assert!(!connected.to_string().contains(&token), "{connected}");
        let id = connected["credential"]["id"].clone();

        assert_eq!(listed(&server, &ada).await.len(), 1);
        assert!(
            listed(&server, &grace).await.is_empty(),
            "grace sees none of ada's"
        );
        assert!(
            listed(&server, &Caller::default()).await.is_empty(),
            "nor does the host"
        );
        let theirs = serde_json::from_value(id).unwrap();
        let refused = server
            .request_as(&grace, Request::CredentialRemove { credential: theirs })
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::NO_CREDENTIAL, "as for one nobody has");
        assert_eq!(listed(&server, &ada).await.len(), 1, "ada's is still hers");

        assert!(
            signed_in_for(&server, &seen, &ada).await,
            "ada's computer has hers"
        );
        assert!(
            !signed_in_for(&server, &seen, &grace).await,
            "grace's computer has none of ada's"
        );
        server.shutdown().await;
    });
}

#[test]
fn someone_signed_in_where_people_have_no_computer_of_their_own_connects_none() {
    let dir = folder("hosts-people-none");
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.hosts = Some(Arc::new(People::default()));
        let (server, _) = setup.build(&dir).await;
        let told = server.request_as(&account(1), Request::Discover {}).await;
        let told = serde_json::to_value(told.unwrap()).unwrap();
        assert!(told.get("connects").is_none(), "{told}");
        let refused = server
            .request_as(&account(1), Request::CredentialList {})
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED);
        let here =
            serde_json::to_value(server.request(Request::Discover {}).await.unwrap()).unwrap();
        assert_eq!(here["connects"]["git_hosts"], true, "{here}");
        server.shutdown().await;
    });
}
