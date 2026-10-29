//! The organization's people and their roles (`specs/model.md`, "Roles"): whoever signs in
//! through the identity provider is recorded, the local person is the owner, admin methods are
//! refused to Members with a plain sentence, only an Owner touches Owners, every server on one
//! store reads the same roles, and `server/discover` says the caller's role only where there
//! is an organization.

use glave_event::{Auth, AuthMethod, OrgRole, PrincipalId, Profile};
use glave_ports::Caller;
use glave_session::protocol::{ApiError, PersonInfo, Reply, Request, code};
use glave_session::signin::person;

use crate::support::*;

const ISSUER: &str = "https://id.example.com";

/// Someone signed in through the identity provider as `subject`, in `groups`.
fn signed_in(subject: &str, groups: &[&str]) -> Caller {
    Caller {
        person: Some(id(subject)),
        auth: Some(Auth {
            method: AuthMethod::Oidc,
            issuer: Some(ISSUER.to_owned()),
            subject: Some(subject.to_owned()),
            session: None,
        }),
        trace: None,
        profile: Some(Profile {
            name: Some(format!("{subject} of the provider")),
            email: None,
            groups: groups.iter().map(|group| (*group).to_owned()).collect(),
        }),
        org: None,
    }
}

fn id(subject: &str) -> PrincipalId {
    person(ISSUER, subject).unwrap()
}

/// The person on the server's computer, signed in as a browser there is.
fn local_browser() -> Caller {
    Caller {
        auth: Some(Auth {
            method: AuthMethod::Local,
            issuer: None,
            subject: None,
            session: None,
        }),
        ..Caller::default()
    }
}

/// A server for an organization on `log`, the `seed`th on it.
async fn organization(dir: &std::path::Path, log: &Shared, seed: u64) -> Tested<Shared> {
    let mut setup = Setup::new(Vec::new());
    setup.identity_provider = true;
    setup.seed = seed;
    setup.build_on(dir, log.clone()).await.0
}

async fn people(server: &Tested<Shared>, caller: &Caller) -> Result<Vec<PersonInfo>, ApiError> {
    let listed = Request::PeopleList {
        after: None,
        limit: None,
    };
    match server.request_as(caller, listed).await? {
        Reply::People { people } => Ok(people),
        other => panic!("people, not {other:?}"),
    }
}

async fn set(
    server: &Tested<Shared>,
    caller: &Caller,
    subject: &str,
    role: OrgRole,
) -> Result<PersonInfo, ApiError> {
    let person = id(subject);
    match server
        .request_as(caller, Request::RoleSet { person, role })
        .await?
    {
        Reply::Person { person } => Ok(person),
        other => panic!("a person, not {other:?}"),
    }
}

async fn role(server: &Tested<Shared>, caller: &Caller) -> Option<OrgRole> {
    match server
        .request_as(caller, Request::Discover {})
        .await
        .unwrap()
    {
        Reply::Server(info) => info.role,
        other => panic!("server info, not {other:?}"),
    }
}

/// Every way a caller's admin call is refused says the same, plainly.
fn refused_to_members(refused: &ApiError) {
    assert_eq!(refused.code, code::NOT_ALLOWED, "{refused:?}");
    assert_eq!(
        refused.message.to_string(),
        "only your organization's admins can do this"
    );
}

#[test]
fn a_member_may_not_list_people_or_give_roles_and_nothing_is_recorded() {
    let dir = folder("people-members");
    block_on!(async {
        let server = organization(&dir, &Shared::default(), 1).await;
        let ada = signed_in("ada", &[]);
        let alan = signed_in("alan", &[]);
        refused_to_members(&people(&server, &ada).await.unwrap_err());
        for role in [OrgRole::Member, OrgRole::Admin, OrgRole::Owner] {
            refused_to_members(&set(&server, &ada, "alan", role).await.unwrap_err());
            refused_to_members(&set(&server, &ada, "ada", role).await.unwrap_err());
        }
        // Alan signs in, and is still a Member, as Ada is.
        refused_to_members(&people(&server, &alan).await.unwrap_err());
        let listed = people(&server, &Caller::default()).await.unwrap();
        let roles: Vec<_> = listed
            .iter()
            .map(|p| (p.subject.as_str(), p.role))
            .collect();
        assert_eq!(roles.len(), 2);
        assert!(roles.contains(&("ada", OrgRole::Member)), "{roles:?}");
        assert!(roles.contains(&("alan", OrgRole::Member)), "{roles:?}");
    });
}

#[test]
fn the_local_person_owns_the_organization_and_gives_the_first_roles() {
    let dir = folder("people-owner");
    block_on!(async {
        let server = organization(&dir, &Shared::default(), 1).await;
        let ada = signed_in("ada", &["engineering"]);
        assert_eq!(role(&server, &ada).await, Some(OrgRole::Member));
        assert_eq!(role(&server, &local_browser()).await, Some(OrgRole::Owner));
        let listed = people(&server, &Caller::default()).await.unwrap();
        assert_eq!(listed.len(), 1, "{listed:?}");
        let first = &listed[0];
        assert_eq!(first.id, id("ada"));
        assert_eq!(first.name.as_deref(), Some("ada of the provider"));
        assert_eq!(first.groups, ["engineering"]);
        let made = set(&server, &Caller::default(), "ada", OrgRole::Admin).await;
        assert_eq!(made.unwrap().role, OrgRole::Admin);
        assert_eq!(role(&server, &ada).await, Some(OrgRole::Admin));
        assert_eq!(people(&server, &ada).await.unwrap().len(), 1);
    });
}

#[test]
fn only_an_owner_makes_an_owner_or_changes_one() {
    let dir = folder("people-owners");
    block_on!(async {
        let server = organization(&dir, &Shared::default(), 1).await;
        let (ada, alan, bea) = (
            signed_in("ada", &[]),
            signed_in("alan", &[]),
            signed_in("bea", &[]),
        );
        for caller in [&ada, &alan, &bea] {
            role(&server, caller).await;
        }
        set(&server, &Caller::default(), "ada", OrgRole::Admin)
            .await
            .unwrap();
        // An Admin gives Admin and Member.
        set(&server, &ada, "alan", OrgRole::Admin).await.unwrap();
        set(&server, &ada, "alan", OrgRole::Member).await.unwrap();
        let refused = set(&server, &ada, "alan", OrgRole::Owner)
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED);
        assert_eq!(
            refused.message.to_string(),
            "only an owner can make someone an owner, or change an owner's role"
        );
        set(&server, &Caller::default(), "bea", OrgRole::Owner)
            .await
            .unwrap();
        let refused = set(&server, &ada, "bea", OrgRole::Member)
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED);
        // An Owner signed in from elsewhere makes another, and one can step down.
        set(&server, &bea, "alan", OrgRole::Owner).await.unwrap();
        set(&server, &alan, "bea", OrgRole::Admin).await.unwrap();
        assert_eq!(role(&server, &bea).await, Some(OrgRole::Admin));
        assert_eq!(role(&server, &alan).await, Some(OrgRole::Owner));
    });
}

#[test]
fn a_role_is_given_only_to_someone_who_signed_in() {
    let dir = folder("people-unknown");
    block_on!(async {
        let server = organization(&dir, &Shared::default(), 1).await;
        let refused = set(&server, &Caller::default(), "nobody", OrgRole::Admin)
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::NO_PERSON);
        assert_eq!(people(&server, &Caller::default()).await.unwrap(), []);
    });
}

#[test]
fn a_person_is_recorded_again_only_when_their_token_says_something_new() {
    let dir = folder("people-again");
    block_on!(async {
        let log = Shared::default();
        let server = organization(&dir, &log, 1).await;
        let read = || async {
            let scope = glave_ports::Scope::of_organization(scope().org());
            let ledger = glave_ports::stream::Reserved::People.id();
            glave_ports::EventLog::read(&log, &scope, ledger, None, usize::MAX)
                .await
                .unwrap()
                .len()
        };
        role(&server, &signed_in("ada", &["a"])).await;
        role(&server, &signed_in("ada", &["a"])).await;
        assert_eq!(read().await, 1);
        role(&server, &signed_in("ada", &["a", "b"])).await;
        assert_eq!(read().await, 2);
        let listed = people(&server, &Caller::default()).await.unwrap();
        assert_eq!(listed[0].groups, ["a", "b"]);
    });
}

#[test]
fn every_server_on_one_store_reads_the_same_roles() {
    let (one, two) = (folder("people-one"), folder("people-two"));
    block_on!(async {
        let log = Shared::default();
        let first = organization(&one, &log, 1).await;
        let second = organization(&two, &log, 2).await;
        let ada = signed_in("ada", &[]);
        role(&first, &ada).await;
        set(&second, &Caller::default(), "ada", OrgRole::Admin)
            .await
            .unwrap();
        assert_eq!(role(&first, &ada).await, Some(OrgRole::Admin));
    });
}

#[test]
fn a_server_without_an_organization_says_no_role() {
    let dir = folder("people-alone");
    block_on!(async {
        let (server, _) = Setup::new(Vec::new())
            .build_on(&dir, Shared::default())
            .await;
        assert_eq!(role(&server, &local_browser()).await, None);
        let organization_dir = folder("people-not-signed-in");
        let organization = organization(&organization_dir, &Shared::default(), 1).await;
        // Over HTTP, `server/discover` without a sign-in is answered for no one.
        assert_eq!(role(&organization, &Caller::default()).await, None);
    });
}
