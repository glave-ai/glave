//! The organization's credits through the session API (`specs/gateway.md`, "Credits"): a
//! server that keeps none says so; whoever runs it adds credits, each payment once, to an
//! organization it serves; its Owners and Admins see the whole account and everyone's
//! spending, anyone else their own; only an Owner sets the monthly limit; and an Owner of
//! another organization neither sees nor adds to these credits.

use glave_event::{Auth, AuthMethod, Currency, Money, OrgRole, PrincipalId};
use glave_ports::{Caller, OrgId};
use glave_session::protocol::{ApiError, CreditsInfo, Reply, Request, code};
use glave_session::signin::person;

use crate::support::*;

const ISSUER: &str = "https://id.example.com";

fn dollars(micros: i64) -> Money {
    Money {
        micros,
        currency: Currency::USD,
    }
}

/// Someone signed in through the identity provider as `subject`.
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

async fn credits(
    server: &Tested<Shared>,
    caller: &Caller,
    request: Request,
) -> Result<CreditsInfo, ApiError> {
    match server.request_as(caller, request).await? {
        Reply::Credits { credits } => Ok(credits),
        other => panic!("credits, not {other:?}"),
    }
}

fn add(micros: i64, reference: &str) -> Request {
    Request::CreditsAdd {
        amount: dollars(micros),
        reference: reference.into(),
        organization: None,
    }
}

#[test]
fn a_server_that_keeps_no_credits_says_so() {
    let dir = folder("credits-none");
    block_on!(async {
        let (server, _) = Setup::new(Vec::new()).build(&dir).await;
        let refused = server.request(Request::CreditsGet {}).await.unwrap_err();
        assert_eq!(refused.code, code::NOT_AVAILABLE);
        assert!(refused.message.to_string().contains("keeps no credits"));
        let Reply::Server(info) = server.request(Request::Discover {}).await.unwrap() else {
            panic!("server info")
        };
        assert!(!info.exists.credits);
    });
}

#[test]
fn whoever_runs_the_server_adds_credits_and_each_role_sees_what_it_may() {
    let dir = folder("credits-roles");
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.identity_provider = true;
        let limit = Some(dollars(10_000_000));
        let (server, _) = setup
            .build_with_credits(&dir, Shared::default(), limit)
            .await;
        let Reply::Server(info) = server.request(Request::Discover {}).await.unwrap() else {
            panic!("server info")
        };
        assert!(info.exists.credits);
        let local = Caller::default();
        let added = credits(&server, &local, add(1_000_000, "order-1"))
            .await
            .unwrap();
        let account = added.account.unwrap();
        assert_eq!(account.balance, dollars(1_000_000));
        assert_eq!(account.limit, limit);
        let again = credits(&server, &local, add(1_000_000, "order-1"))
            .await
            .unwrap();
        assert_eq!(
            again.account.unwrap().balance,
            dollars(1_000_000),
            "once a payment"
        );
        let (ada, alan) = (signed_in("ada"), signed_in("alan"));
        let mine = credits(&server, &ada, Request::CreditsGet {})
            .await
            .unwrap();
        assert_eq!(mine.account, None, "a Member sees no account");
        assert_eq!(mine.people.len(), 1);
        assert!(mine.people[0].you && mine.people[0].spent == dollars(0));
        assert!(!mine.can_set_limit);
        let refused = credits(&server, &ada, add(5, "order-2")).await.unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED);
        assert!(
            refused
                .message
                .to_string()
                .contains("only whoever runs this glave")
        );
        let made = Request::RoleSet {
            person: id("alan"),
            role: OrgRole::Admin,
        };
        credits(&server, &alan, Request::CreditsGet {})
            .await
            .unwrap();
        server.request_as(&local, made).await.unwrap();
        let admin = credits(&server, &alan, Request::CreditsGet {})
            .await
            .unwrap();
        assert_eq!(admin.account.unwrap().balance, dollars(1_000_000));
        assert!(!admin.can_set_limit);
        let raise = Request::CreditsLimit {
            limit: dollars(25_000_000),
        };
        let refused = credits(&server, &alan, raise.clone()).await.unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED);
        assert!(refused.message.to_string().contains("only an owner"));
        let raised = credits(&server, &local, raise).await.unwrap();
        assert_eq!(raised.account.unwrap().limit, Some(dollars(25_000_000)));
    });
}

#[test]
fn credits_are_dollars_for_a_payment_named_into_an_organization_the_server_serves() {
    let dir = folder("credits-refused");
    block_on!(async {
        let (server, _) = Setup::new(Vec::new())
            .build_with_credits(&dir, Shared::default(), None)
            .await;
        let local = Caller::default();
        for (wrong, expected) in [
            (add(0, "order-1"), code::INVALID_PARAMS),
            (add(5, " "), code::INVALID_PARAMS),
            (add(5, &"x".repeat(257)), code::INVALID_PARAMS),
            (
                Request::CreditsAdd {
                    amount: dollars(5),
                    reference: "order-1".into(),
                    organization: Some(OrgId::from_uuid(uuid::Uuid::from_u128(99))),
                },
                code::NO_ORGANIZATION,
            ),
        ] {
            let refused = credits(&server, &local, wrong).await.unwrap_err();
            assert_eq!(refused.code, expected, "{refused:?}");
        }
        let euros = Request::CreditsLimit {
            limit: Money {
                micros: 5,
                currency: Currency::new("EUR").unwrap(),
            },
        };
        let refused = credits(&server, &local, euros).await.unwrap_err();
        assert_eq!(refused.code, code::INVALID_PARAMS);
    });
}

#[test]
fn an_owner_of_another_organization_neither_sees_nor_adds_to_these_credits() {
    let dir = folder("credits-orgs");
    block_on!(async {
        let log = Shared::default();
        let limit = Some(dollars(10_000_000));
        let mut ours = Setup::new(Vec::new());
        ours.identity_provider = true;
        let (ours, _) = ours.build_with_credits(&dir, log.clone(), limit).await;
        let mut theirs = Setup::new(Vec::new());
        theirs.identity_provider = true;
        theirs.seed = 2;
        theirs.home = glave_ports::Scope::glaves_own(
            theirs_org(),
            glave_ports::ProjectId::from_uuid(uuid::Uuid::from_u128(42)),
        );
        let (theirs, _) = theirs.build_with_credits(&dir, log, limit).await;
        let local = Caller::default();
        credits(&ours, &local, add(1_000_000, "order-1"))
            .await
            .unwrap();
        let olga = signed_in("olga");
        credits(&theirs, &olga, Request::CreditsGet {})
            .await
            .unwrap();
        let owner = Request::RoleSet {
            person: id("olga"),
            role: OrgRole::Owner,
        };
        theirs.request_as(&local, owner).await.unwrap();
        let seen = credits(&theirs, &olga, Request::CreditsGet {})
            .await
            .unwrap();
        assert_eq!(seen.account.unwrap().balance, dollars(0), "their own alone");
        let into_ours = Request::CreditsAdd {
            amount: dollars(5),
            reference: "order-2".into(),
            organization: Some(scope().org()),
        };
        let refused = credits(&theirs, &olga, into_ours).await.unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED);
        // Whoever runs the server adds to the other organization by its id.
        let into_theirs = Request::CreditsAdd {
            amount: dollars(5_000_000),
            reference: "order-3".into(),
            organization: Some(theirs_org()),
        };
        let added = credits(&ours, &local, into_theirs).await.unwrap();
        assert_eq!(added.account.unwrap().balance, dollars(5_000_000));
        assert!(!added.can_set_limit, "its limit is its Owner's to set");
        let seen = credits(&theirs, &olga, Request::CreditsGet {})
            .await
            .unwrap();
        assert_eq!(seen.account.unwrap().balance, dollars(5_000_000));
        let ours_now = credits(&ours, &local, Request::CreditsGet {})
            .await
            .unwrap();
        assert_eq!(ours_now.account.unwrap().balance, dollars(1_000_000));
    });
}

/// The other organization of the test that has two.
fn theirs_org() -> OrgId {
    OrgId::from_uuid(uuid::Uuid::from_u128(41))
}
