//! Taking someone out of an organization (`specs/model.md`, "Identity"): one admin action ends,
//! at once, the watches they hold there and refuses their next request; signing in again
//! through the company login does not bring them back, an invite does, on every server on the
//! store. Only admins remove, only an Owner removes an Owner, and someone not there is no one
//! to remove.

use glave_event::{Auth, AuthMethod, OrgRole, PrincipalId, Secret};
use glave_ports::Caller;
use glave_session::Watched;
use glave_session::protocol::{ApiError, Reply, Request, code};
use glave_session::signin::person;

use crate::support::*;

const ISSUER: &str = "https://id.example.com";

/// Someone signed in through the company login as `subject`.
fn signed_in(subject: &str) -> Caller {
    Caller {
        person: Some(id(subject)),
        auth: Some(Auth {
            method: AuthMethod::Oidc,
            issuer: Some(ISSUER.to_owned()),
            subject: Some(subject.to_owned()),
            session: None,
        }),
        ..Caller::default()
    }
}

fn id(subject: &str) -> PrincipalId {
    person(ISSUER, subject).unwrap()
}

/// A server whose people sign in through a company login.
async fn organization(dir: &std::path::Path) -> Tested {
    let mut setup = Setup::new(Vec::new());
    setup.identity_provider = true;
    setup.build(dir).await.0
}

/// The `seed`th server of one organization on `log`.
async fn on(dir: &std::path::Path, log: &Shared, seed: u64) -> Tested<Shared> {
    let mut setup = Setup::new(Vec::new());
    setup.identity_provider = true;
    setup.seed = seed;
    setup.build_on(dir, log.clone()).await.0
}

async fn remove<L: glave_ports::EventLog + 'static>(
    server: &Tested<L>,
    caller: &Caller,
    subject: &str,
) -> Result<Reply, ApiError> {
    let person = id(subject);
    server
        .request_as(caller, Request::PersonRemove { person })
        .await
}

fn list() -> Request {
    Request::ThreadList {
        project: None,
        limit: None,
        show: None,
        cursor: None,
    }
}

#[test]
fn someone_removed_has_their_watches_end_at_once_and_comes_back_only_by_an_invite() {
    block_on!(async {
        let dir = folder("remove-person");
        let server = organization(&dir).await;
        let ada = signed_in("ada");
        server.request_as(&ada, list()).await.unwrap();
        let mut watching = server.watch_list(&ada, None, None).await.unwrap();
        let owner = Caller::default();
        let (came, removed) = tokio::join!(watching.next(), remove(&server, &owner, "ada"));
        removed.unwrap();
        let Some(Watched::Ended(Err(ended))) = came else {
            panic!("the watch ended, not {came:?}");
        };
        assert_eq!(ended.code, code::NO_SIGN_IN);
        assert!(watching.next().await.is_none(), "nothing more after");

        // Signing in again through the company login lets her in to nothing.
        let refused = server.request_as(&ada, list()).await.unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED);
        assert!(
            refused.message.to_string().contains("invite"),
            "{refused:?}"
        );
        let people = Request::PeopleList {
            after: None,
            limit: None,
        };
        let Ok(Reply::People { people }) = server.request(people).await else {
            panic!("people");
        };
        assert!(people.iter().all(|person| person.id != id("ada")));

        // An invite brings her back, with its role.
        let invite = Request::InviteCreate {
            role: OrgRole::Admin,
        };
        let Ok(Reply::Invited { code, .. }) = server.request(invite).await else {
            panic!("an invite");
        };
        let accept = Request::InviteAccept {
            code: Secret::new(code.expose()),
        };
        let Ok(Reply::Person { person }) = server.request_as(&ada, accept).await else {
            panic!("back");
        };
        assert_eq!(person.role, OrgRole::Admin);
        server.request_as(&ada, list()).await.unwrap();
    });
}

#[test]
fn only_admins_remove_and_only_owners_remove_owners() {
    block_on!(async {
        let dir = folder("remove-who");
        let server = organization(&dir).await;
        let (ada, alan, grace) = (signed_in("ada"), signed_in("alan"), signed_in("grace"));
        for caller in [&ada, &alan, &grace] {
            server.request_as(caller, list()).await.unwrap();
        }
        let owner = Caller::default();
        for (person, role) in [("ada", OrgRole::Admin), ("grace", OrgRole::Owner)] {
            let set = Request::RoleSet {
                person: id(person),
                role,
            };
            server.request_as(&owner, set).await.unwrap();
        }
        let refused = remove(&server, &alan, "ada").await.unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED, "a Member removes no one");
        let refused = remove(&server, &ada, "grace").await.unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED, "an Admin removes no Owner");
        let nobody = remove(&server, &ada, "linus").await.unwrap_err();
        assert_eq!(nobody.code, code::NO_PERSON);
        remove(&server, &ada, "alan").await.unwrap();
        remove(&server, &grace, "ada").await.unwrap();
        // Asked again, it is done again.
        remove(&server, &grace, "ada").await.unwrap();
    });
}

#[test]
fn another_server_on_the_store_refuses_them_at_their_next_request() {
    block_on!(async {
        let (first, second) = (folder("remove-first"), folder("remove-second"));
        let log = Shared::default();
        let one = on(&first, &log, 1).await;
        let two = on(&second, &log, 2).await;
        let ada = signed_in("ada");
        two.request_as(&ada, list()).await.unwrap();
        remove(&one, &Caller::default(), "ada").await.unwrap();
        let refused = two.request_as(&ada, list()).await.unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED);
    });
}
