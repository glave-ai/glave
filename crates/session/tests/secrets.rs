//! Secrets through the session API: the person on this computer adds, lists, replaces and
//! removes them; no answer ever holds a value; each is kept sealed under a key of its own, so
//! one removed or replaced no longer opens in the store. Anyone else is refused, and so is
//! everyone where the server keeps none.

use std::path::Path;
use std::sync::Arc;

use glave_event::{Auth, AuthMethod, Body, PrincipalId, Secret, decode};
use glave_keys::{InMemoryRoot, KeySettings};
use glave_ports::stream::Reserved;
use glave_ports::testing::any_secret;
use glave_ports::{Caller, EventLog, Scope};
use glave_session::Answering;
use glave_session::protocol::{Request, code};
use glave_store_sqlite::{SqliteLog, Store};

use crate::support::*;

fn add(name: &str, hosts: &[&str], value: &str) -> Request {
    Request::SecretAdd {
        name: name.into(),
        hosts: hosts.iter().map(|host| (*host).to_owned()).collect(),
        value: Secret::new(value),
    }
}

/// A server in `dir` keeping its person's secrets in their personal project, on a sealed
/// SQLite store, with the store and that project.
async fn keeping(dir: &Path) -> (Tested<SqliteLog>, Store, Scope) {
    let root = InMemoryRoot::new().unwrap();
    let store = Store::open_in_memory(KeySettings::new(Arc::new(root))).unwrap();
    let person = PrincipalId::new(jiff::Timestamp::UNIX_EPOCH, [5; 10]).unwrap();
    let mut setup = Setup::new(Vec::new());
    let personal = setup.home;
    setup.secrets = Some(KeepSecrets::new(store.keyring(), personal, person));
    let (server, _) = setup.build_with_credits(dir, store.log(), None).await;
    (server, store, personal)
}

#[test]
fn a_value_never_shows_in_a_request_s_debug() {
    let value = any_secret();
    let shown = format!("{:?}", add("STRIPE_KEY", &["api.stripe.com"], &value));
    assert!(!shown.contains(&value), "{shown}");
}

#[test]
fn the_person_here_keeps_secrets_sealed_and_never_sees_a_value_again() {
    let dir = folder("secrets-kept");
    block_on!(async {
        let (server, store, personal) = keeping(&dir).await;
        let (first, second) = (any_secret(), any_secret());

        let refused = [
            add("1KEY", &["api.stripe.com"], &first),
            add("STRIPE_KEY", &["https://api.stripe.com"], &first),
            add("STRIPE_KEY", &[], &first),
            add("STRIPE_KEY", &["api.stripe.com"], "short"),
        ];
        for request in refused {
            let error = server.request(request).await.unwrap_err();
            assert_eq!(error.code, code::INVALID_PARAMS, "{error}");
        }
        let mut answers = Vec::new();
        answers.push(
            server
                .request(add("STRIPE_KEY", &["api.stripe.com"], &first))
                .await,
        );
        let again = server.request(add("STRIPE_KEY", &["x.com"], &second)).await;
        assert_eq!(again.as_ref().unwrap_err().code, code::INVALID_PARAMS);
        let replace = Request::SecretReplace {
            name: "STRIPE_KEY".into(),
            hosts: Some(vec!["*.stripe.com".into()]),
            value: Secret::new(second.clone()),
        };
        answers.push(server.request(replace).await);
        answers.push(server.request(Request::SecretList {}).await);
        let shown = serde_json::to_value(answers.last().unwrap().as_ref().unwrap()).unwrap();
        let listed = serde_json::json!([{"name": "STRIPE_KEY", "hosts": ["*.stripe.com"]}]);
        assert_eq!(shown["secrets"], listed);

        // The replaced value no longer opens in the store; the one kept does, sealed.
        let ledger = Reserved::Accounts.id();
        let kept = values(&store.log(), personal, ledger).await;
        assert_eq!(kept, std::slice::from_ref(&second));

        let remove = Request::SecretRemove {
            name: "STRIPE_KEY".into(),
        };
        answers.push(server.request(remove.clone()).await);
        let gone = server.request(remove).await.unwrap_err();
        assert_eq!(gone.code, code::NO_CREDENTIAL);
        answers.push(server.request(Request::SecretList {}).await);
        assert_eq!(values(&store.log(), personal, ledger).await, [""; 0]);
        let shown = format!("{answers:?}");
        for value in [&first, &second] {
            assert!(!shown.contains(value.as_str()), "{shown}");
        }
        server.shutdown().await;
    });
}

/// The values of the secrets whose events still open in `ledger`.
async fn values<L: EventLog>(log: &L, scope: Scope, ledger: glave_event::ThreadId) -> Vec<String> {
    let events = log.read(&scope, ledger, None, 1_000).await.unwrap();
    events
        .into_iter()
        .filter_map(|raw| match decode(raw).ok()?.body {
            Body::SecretAdded(added) => Some(added.value.expose().to_owned()),
            _ => None,
        })
        .collect()
}

#[test]
fn no_one_else_reaches_secrets_and_a_server_without_them_refuses_everyone() {
    let dir = folder("secrets-refused");
    block_on!(async {
        let (server, _store, _) = keeping(&dir).await;
        let person = PrincipalId::new(jiff::Timestamp::UNIX_EPOCH, [5; 10]).unwrap();
        for method in [AuthMethod::Oidc, AuthMethod::Account] {
            let elsewhere = Caller {
                person: Some(person),
                auth: Some(Auth {
                    method,
                    issuer: None,
                    subject: None,
                    session: None,
                }),
                ..Caller::default()
            };
            for request in [
                add("STRIPE_KEY", &["api.stripe.com"], &any_secret()),
                Request::SecretList {},
            ] {
                let refused = server.request_as(&elsewhere, request).await.unwrap_err();
                assert_eq!(refused.code, code::NOT_ALLOWED, "{refused}");
            }
        }
        server.shutdown().await;

        let dir = folder("secrets-none");
        let none = crate::support::server(&dir, Vec::new(), Answering::Clients).await;
        let refused = none.request(Request::SecretList {}).await.unwrap_err();
        assert_eq!(refused.code, code::NOT_AVAILABLE);
        none.shutdown().await;
    });
}
