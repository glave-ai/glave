//! Invites to an organization (`specs/model.md`, "Invites"): a one-time link that gives a role,
//! opened by someone signed in through the identity provider. A code works once, on any server
//! of the store, and for 7 days; a used, revoked, expired, guessed or another organization's
//! code is refused alike; only those whose role permits inviting make, list and revoke them; an
//! invite never lowers a role, and gives no more than its maker may still give.

use std::sync::Arc;

use glave_event::{Auth, AuthMethod, OrgRole, PrincipalId, Secret};
use glave_ports::testing::ManualClock;
use glave_ports::{Caller, OrgId, ProjectId, Scope};
use glave_session::protocol::{ApiError, InviteInfo, PersonInfo, Reply, Request, code};
use glave_session::signin::person;

use crate::support::*;

const ISSUER: &str = "https://id.example.com";

/// When the tests' clock starts.
const START: i64 = 1_791_001_000_000;

/// An invite's lifetime, in milliseconds.
const WEEK: i64 = 7 * 24 * 60 * 60 * 1_000;

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

/// The local person, the organization's owner.
fn owner() -> Caller {
    Caller::default()
}

/// A server for organization `org` on `log`, the `seed`th on it, reading `clock`, serving its
/// one local person.
async fn organization(
    dir: &std::path::Path,
    log: &Shared,
    org: u128,
    seed: u64,
    clock: &ManualClock,
) -> Tested<Shared> {
    let mut setup = Setup::new(Vec::new());
    setup.identity_provider = true;
    setup.seed = seed;
    setup.clock = clock.clone();
    // The store keeps one local person, whom every server on it serves.
    setup.settings.person = Some(
        PrincipalId::try_from(uuid::Uuid::from_u128(
            0x0192_f5c0_0000_7000_8000_0000_0000_0100 + org,
        ))
        .unwrap(),
    );
    setup.home = Scope::glaves_own(
        OrgId::from_uuid(uuid::Uuid::from_u128(org)),
        ProjectId::from_uuid(uuid::Uuid::from_u128(0x10 + org)),
    );
    setup.build_on(dir, log.clone()).await.0
}

async fn invite(
    server: &Tested<Shared>,
    caller: &Caller,
    role: OrgRole,
) -> Result<(InviteInfo, Secret), ApiError> {
    match server
        .request_as(caller, Request::InviteCreate { role, email: None })
        .await?
    {
        Reply::Invited { invite, code } => Ok((invite, code)),
        other => panic!("an invite, not {other:?}"),
    }
}

async fn accept(
    server: &Tested<Shared>,
    caller: &Caller,
    code: &Secret,
) -> Result<PersonInfo, ApiError> {
    let code = Secret::new(code.expose().to_owned());
    match server
        .request_as(caller, Request::InviteAccept { code })
        .await?
    {
        Reply::Person { person } => Ok(person),
        other => panic!("a person, not {other:?}"),
    }
}

async fn waiting(server: &Tested<Shared>, caller: &Caller) -> Result<Vec<InviteInfo>, ApiError> {
    match server.request_as(caller, Request::InviteList {}).await? {
        Reply::Invites { invites } => Ok(invites),
        other => panic!("invites, not {other:?}"),
    }
}

/// `caller` signs in, and so is recorded, as anyone's first request records them.
async fn sign_in(server: &Tested<Shared>, caller: &Caller) {
    server
        .request_as(caller, Request::Discover {})
        .await
        .unwrap();
}

async fn set(server: &Tested<Shared>, caller: &Caller, subject: &str, role: OrgRole) {
    let person = id(subject);
    server
        .request_as(caller, Request::RoleSet { person, role })
        .await
        .unwrap();
}

/// Every code that opens nothing is refused the same way, saying nothing of why.
fn refused_alike(refused: &ApiError) {
    assert_eq!(refused.code, code::NO_INVITE, "{refused:?}");
    assert_eq!(
        refused.message.to_string(),
        "that invite link was used already, revoked or has expired: ask whoever sent it for a new one"
    );
}

#[test]
fn an_invite_gives_its_role_once_and_is_listed_until_then_without_its_code() {
    let dir = folder("invites-once");
    block_on!(async {
        let clock = ManualClock::at_millis(START);
        let server = organization(&dir, &Shared::default(), 1, 1, &clock).await;
        let (made, code) = invite(&server, &owner(), OrgRole::Admin).await.unwrap();
        assert_eq!(made.role, OrgRole::Admin);
        assert_eq!(made.expires.timestamp().as_millisecond(), START + WEEK);
        let listed = waiting(&server, &owner()).await.unwrap();
        assert_eq!(listed, std::slice::from_ref(&made));
        let shown = serde_json::to_string(&listed).unwrap();
        assert!(!shown.contains(code.expose()), "{shown}");
        assert!(!shown.contains("hash"), "{shown}");

        let ada = accept(&server, &signed_in("ada"), &code).await.unwrap();
        assert_eq!((ada.id, ada.role), (id("ada"), OrgRole::Admin));
        // Spent: no one opens it again, Ada included, and it waits no more.
        refused_alike(
            &accept(&server, &signed_in("alan"), &code)
                .await
                .unwrap_err(),
        );
        refused_alike(&accept(&server, &signed_in("ada"), &code).await.unwrap_err());
        assert_eq!(waiting(&server, &owner()).await.unwrap(), []);
        // A guessed code is refused the same way.
        let guessed = Secret::new("0".repeat(64));
        refused_alike(
            &accept(&server, &signed_in("alan"), &guessed)
                .await
                .unwrap_err(),
        );
    });
}

#[test]
fn an_invite_expires_after_seven_days() {
    let dir = folder("invites-expire");
    block_on!(async {
        let clock = ManualClock::at_millis(START);
        let server = organization(&dir, &Shared::default(), 1, 1, &clock).await;
        let (_, early) = invite(&server, &owner(), OrgRole::Member).await.unwrap();
        let (_, late) = invite(&server, &owner(), OrgRole::Admin).await.unwrap();
        clock.set_millis(START + WEEK - 1);
        accept(&server, &signed_in("ada"), &early).await.unwrap();
        clock.set_millis(START + WEEK);
        refused_alike(
            &accept(&server, &signed_in("alan"), &late)
                .await
                .unwrap_err(),
        );
        assert_eq!(waiting(&server, &owner()).await.unwrap(), []);
    });
}

#[test]
fn only_those_whose_role_permits_inviting_make_list_and_revoke_invites() {
    let dir = folder("invites-who");
    block_on!(async {
        let clock = ManualClock::at_millis(START);
        let server = organization(&dir, &Shared::default(), 1, 1, &clock).await;
        let (ada, alan) = (signed_in("ada"), signed_in("alan"));
        let (made, _) = invite(&server, &owner(), OrgRole::Member).await.unwrap();
        // Alan, a Member, may do none of it.
        let admins_only = "only your organization's admins can do this";
        let refusals = [
            invite(&server, &alan, OrgRole::Member).await.unwrap_err(),
            waiting(&server, &alan).await.unwrap_err(),
            server
                .request_as(&alan, Request::InviteRevoke { invite: made.id })
                .await
                .unwrap_err(),
        ];
        for refused in refusals {
            assert_eq!(refused.code, code::NOT_ALLOWED, "{refused:?}");
            assert_eq!(refused.message.to_string(), admins_only);
        }
        // Ada, an Admin, invites Admins and Members, never an Owner.
        sign_in(&server, &ada).await;
        set(&server, &owner(), "ada", OrgRole::Admin).await;
        invite(&server, &ada, OrgRole::Admin).await.unwrap();
        let refused = invite(&server, &ada, OrgRole::Owner).await.unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED);
        let (_, owners) = invite(&server, &owner(), OrgRole::Owner).await.unwrap();
        assert_eq!(waiting(&server, &ada).await.unwrap().len(), 3);
        // She revokes one; it opens nothing after, and revoking it again finds nothing.
        server
            .request_as(&ada, Request::InviteRevoke { invite: made.id })
            .await
            .unwrap();
        let again = server
            .request_as(&ada, Request::InviteRevoke { invite: made.id })
            .await
            .unwrap_err();
        assert_eq!(again.code, code::NO_INVITE);
        assert_eq!(waiting(&server, &ada).await.unwrap().len(), 2);
        let bea = accept(&server, &signed_in("bea"), &owners).await.unwrap();
        assert_eq!(bea.role, OrgRole::Owner);
    });
}

#[test]
fn a_revoked_invite_opens_nothing() {
    let dir = folder("invites-revoked");
    block_on!(async {
        let clock = ManualClock::at_millis(START);
        let server = organization(&dir, &Shared::default(), 1, 1, &clock).await;
        let (made, code) = invite(&server, &owner(), OrgRole::Admin).await.unwrap();
        server
            .request_as(&owner(), Request::InviteRevoke { invite: made.id })
            .await
            .unwrap();
        refused_alike(&accept(&server, &signed_in("ada"), &code).await.unwrap_err());
    });
}

#[test]
fn an_invite_never_lowers_a_role_nor_gives_more_than_its_maker_still_may() {
    let dir = folder("invites-roles");
    block_on!(async {
        let clock = ManualClock::at_millis(START);
        let server = organization(&dir, &Shared::default(), 1, 1, &clock).await;
        let (ada, bea) = (signed_in("ada"), signed_in("bea"));
        // Bea, an Owner, opens a Member's invite and stays an Owner.
        let (_, member) = invite(&server, &owner(), OrgRole::Member).await.unwrap();
        sign_in(&server, &bea).await;
        set(&server, &owner(), "bea", OrgRole::Owner).await;
        assert_eq!(
            accept(&server, &bea, &member).await.unwrap().role,
            OrgRole::Owner
        );
        // Ada invites as an Admin, then is made a Member: her invite gives nothing now.
        sign_in(&server, &ada).await;
        set(&server, &owner(), "ada", OrgRole::Admin).await;
        let (_, hers) = invite(&server, &ada, OrgRole::Admin).await.unwrap();
        set(&server, &owner(), "ada", OrgRole::Member).await;
        refused_alike(
            &accept(&server, &signed_in("alan"), &hers)
                .await
                .unwrap_err(),
        );
    });
}

#[test]
fn a_code_works_once_across_the_servers_of_one_store_and_never_in_another_organization() {
    let (one, two, other) = (
        folder("invites-one"),
        folder("invites-two"),
        folder("invites-other"),
    );
    block_on!(async {
        let log = Shared::default();
        let clock = ManualClock::at_millis(START);
        let first = organization(&one, &log, 1, 1, &clock).await;
        let second = organization(&two, &log, 1, 2, &clock).await;
        let elsewhere = organization(&other, &log, 2, 3, &clock).await;
        let (_, code) = invite(&first, &owner(), OrgRole::Admin).await.unwrap();
        // Another organization on the same store never finds it, and lists none.
        let linus = signed_in("linus");
        refused_alike(&accept(&elsewhere, &linus, &code).await.unwrap_err());
        assert_eq!(waiting(&elsewhere, &owner()).await.unwrap(), []);
        // Another server of this one opens it, and then neither does again.
        let ada = accept(&second, &signed_in("ada"), &code).await.unwrap();
        assert_eq!(ada.role, OrgRole::Admin);
        refused_alike(&accept(&first, &signed_in("alan"), &code).await.unwrap_err());
        refused_alike(
            &accept(&second, &signed_in("alan"), &code)
                .await
                .unwrap_err(),
        );
    });
}

#[test]
fn where_no_one_else_reaches_the_server_there_is_no_one_to_invite_or_accept() {
    let dir = folder("invites-alone");
    block_on!(async {
        let (server, _) = Setup::new(Vec::new())
            .build_on(&dir, Shared::default())
            .await;
        let refused = invite(&server, &owner(), OrgRole::Member)
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED);
        assert_eq!(
            refused.message.to_string(),
            "no one else can reach this glave, so there is no one to invite: give it an address \
             people reach it at first"
        );
        // Only someone signed in through the identity provider opens an invite.
        let guessed = Secret::new("0".repeat(64));
        let refused = accept(&server, &owner(), &guessed).await.unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED);
    });
}

/// An invite named for an address is sent there, its link at the server's public origin; the
/// address is not kept. Without a mail server, or with no address, nothing is sent.
#[test]
fn an_invite_is_emailed_where_the_server_sends_email() {
    let dir = folder("invites-emailed");
    block_on!(async {
        let outbox = Arc::new(glave_ports::testing::Outbox::default());
        let mut setup = Setup::new(Vec::new());
        setup.identity_provider = true;
        setup.origin = glave_ports::host::PublicOrigin::parse("https://glave.test");
        setup.mail = Some(Arc::clone(&outbox) as Arc<dyn glave_ports::Mailer>);
        let (server, _) = setup.build_on(&dir, Shared::default()).await;
        let create = |email: Option<&str>| Request::InviteCreate {
            role: OrgRole::Member,
            email: email.map(str::to_owned),
        };
        let Ok(Reply::Invited { code, .. }) = server.request(create(Some("ada@example.com"))).await
        else {
            panic!("an invite");
        };
        let sent = outbox.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].to, "ada@example.com");
        let link = format!("https://glave.test/#invite={}", code.expose());
        assert!(sent[0].text.contains(&link), "{}", sent[0].text);
        server.request(create(None)).await.unwrap();
        assert_eq!(outbox.sent().len(), 1, "no address, no email");
        let refused = server
            .request(create(Some("not an address")))
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::INVALID_PARAMS);

        let unsent = folder("invites-unsent");
        let mut setup = Setup::new(Vec::new());
        setup.identity_provider = true;
        let (quiet, _) = setup.build_on(&unsent, Shared::default()).await;
        let refused = quiet
            .request(create(Some("ada@example.com")))
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::NOT_AVAILABLE);
    });
}
