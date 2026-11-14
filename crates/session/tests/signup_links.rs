//! Sign-up links whoever runs the server makes (`specs/model.md`, "Identity"): from its
//! computer, while it takes no sign-ups and sends no email, a link to hand to one person, which
//! makes their account, named as they give it and with no address, and their personal space,
//! an organization with them its Owner that takes no invites, as signing up does. It opens once, for a week, on the servers
//! of the store it was made in alone; no one else may make one, and a server that keeps no
//! glave accounts at a public origin says it makes none.

use std::sync::Arc;

use glave_event::Secret;
use glave_identity::testing::Authenticator;
use glave_ports::testing::Outbox;
use glave_session::Answering;
use glave_session::protocol::{LinkKind, Request, code};
use glave_session::signin::{LocalToken, SignIn};
use serde_json::{Value, json};

use crate::signups::{linked, serve, serve_mailless, serve_sharing, signed_up};
use crate::support::issuer::fixed_clock;
use crate::support::web::{
    ORIGIN, Served, Web, asked, elsewhere, make_named_passkey, make_passkey, serve_with,
};
use crate::support::*;

/// Whoever runs `served`, from its computer, with the local token its tests sign in with.
fn operator(served: &Served) -> Web {
    Web::new(served, Some(LocalToken::new([21; 32]).reveal().to_owned()))
}

/// What `signup/link` answers `caller` with: a link, or the refusal's code.
async fn link_for(caller: &mut Web) -> Value {
    asked(caller, &Request::SignupLink {}).await
}

/// The code in a sign-up link `made`.
fn code_of(made: &Value) -> Secret {
    let link = made["link"].as_str().unwrap_or_else(|| panic!("{made}"));
    let marker = format!("{ORIGIN}/#signup=");
    Secret::new(
        link.strip_prefix(&marker)
            .unwrap_or_else(|| panic!("{link}")),
    )
}

/// What making a passkey with a sign-up link's `code` on a new device, its person giving a
/// name, is answered with.
async fn open(served: &Served, code: Secret) -> Value {
    let mut device = Authenticator::new(ORIGIN).unwrap();
    make_named_passkey(served, code, LinkKind::Signup, &mut device, Some("Ada")).await
}

#[test]
fn a_link_signs_someone_up_to_their_own_organization_with_no_sign_up_or_email() {
    block_on!(async {
        let dir = folder("signup-links-made");
        let (served, _) = serve_mailless(&dir).await;
        let mut anyone = elsewhere(&served, None);
        let made = link_for(&mut operator(&served)).await;
        assert!(made["expires"].is_string(), "{made}");
        // Sign-up stays closed to everyone else.
        let told = asked(&mut anyone, &Request::Discover {}).await;
        let ways = told["signin"].as_array().cloned().unwrap_or_default();
        assert!(!ways.contains(&json!("signup")), "{told}");
        let start = Request::SignupStart {
            email: "bea@example.com".into(),
        };
        assert_eq!(asked(&mut anyone, &start).await, json!(code::NOT_AVAILABLE));

        let code = code_of(&made);
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        let signup = LinkKind::Signup;
        let named = Some("Ada Lovelace");
        let opened = make_named_passkey(&served, code.clone(), signup, &mut phone, named).await;
        let mut ada = elsewhere(&served, opened["session"].as_str().map(str::to_owned));
        let listed = asked(&mut ada, &Request::OrganizationList {}).await;
        let orgs = listed["organizations"].as_array().unwrap();
        assert_eq!(orgs.len(), 1, "{listed}");
        assert_eq!(orgs[0]["own"], json!(true));
        assert_eq!(orgs[0]["people"], json!(1));
        assert_eq!(orgs[0]["role"], json!("owner"));
        let people = Request::PeopleList {
            after: None,
            limit: None,
        };
        let people = asked(&mut ada, &people).await;
        let her = &people["people"][0];
        assert_eq!(her["name"], json!("Ada Lovelace"), "{people}");
        assert!(her.get("email").is_none(), "no address: {people}");
        let started: Request =
            serde_json::from_value(json!({"method": "thread/start", "params": {}})).unwrap();
        assert!(asked(&mut ada, &started).await["thread"]["id"].is_string());

        // Once.
        assert_eq!(open(&served, code).await, json!(code::NO_INVITE));
        served.close().await;
    });
}

#[test]
fn a_link_makes_a_personal_space_which_takes_no_invites() {
    block_on!(async {
        let dir = folder("signup-links-personal");
        let (served, _) = serve_mailless(&dir).await;
        let code = code_of(&link_for(&mut operator(&served)).await);
        let opened = open(&served, code).await;
        let mut ada = elsewhere(&served, opened["session"].as_str().map(str::to_owned));
        let told = asked(&mut ada, &Request::Discover {}).await;
        assert_eq!(told["personal"], json!(true), "{told}");
        assert!(told.get("invites").is_none(), "{told}");
        assert_eq!(told["organizations"], json!(true), "{told}");
        let invite = Request::InviteCreate {
            role: glave_event::OrgRole::Member,
            email: None,
        };
        assert_eq!(asked(&mut ada, &invite).await, json!(code::NOT_ALLOWED));
        served.close().await;
    });
}

#[test]
fn a_link_works_for_a_week_and_then_opens_nothing() {
    block_on!(async {
        let dir = folder("signup-links-expire");
        let (served, clock) = serve_mailless(&dir).await;
        let mut operator = operator(&served);
        let kept = code_of(&link_for(&mut operator).await);
        let lapsed = code_of(&link_for(&mut operator).await);
        clock.advance_seconds(7 * 24 * 60 * 60 - 60);
        assert!(open(&served, kept).await["session"].is_string());
        clock.advance_seconds(120);
        assert_eq!(open(&served, lapsed).await, json!(code::NO_INVITE));
        served.close().await;
    });
}

#[test]
fn only_whoever_runs_the_server_makes_one_and_it_opens_no_other_door() {
    block_on!(async {
        let dir = folder("signup-links-who");
        let outbox = Arc::new(Outbox::default());
        let served = serve(&dir, true, &outbox).await;
        let mut ada = signed_up(&served, &outbox, "ada@example.com").await;
        assert_eq!(link_for(&mut ada).await, json!(code::NOT_ALLOWED));
        let mut signed_out = elsewhere(&served, None);
        let refused = signed_out.call(&Request::SignupLink {}).await;
        assert!(
            refused.get("result").is_none(),
            "needs a sign-in: {refused}"
        );
        let code = code_of(&link_for(&mut operator(&served)).await);
        // Not a link to get back in, nor an invite.
        let mut device = Authenticator::new(ORIGIN).unwrap();
        let recovering = make_passkey(&served, code.clone(), LinkKind::Recovery, &mut device);
        assert_eq!(recovering.await, json!(code::NO_INVITE));
        let accept = Request::InviteAccept { code: code.clone() };
        assert!(asked(&mut ada, &accept).await.is_number());
        assert!(open(&served, code).await["session"].is_string());
        served.close().await;
    });
}

#[test]
fn a_link_made_on_one_store_opens_nothing_on_another() {
    block_on!(async {
        let here = folder("signup-links-here");
        let there = folder("signup-links-there");
        let (made_here, _) = serve_mailless(&here).await;
        let (other, _) = serve_mailless(&there).await;
        let code = code_of(&link_for(&mut operator(&made_here)).await);
        assert_eq!(open(&other, code.clone()).await, json!(code::NO_INVITE));
        assert!(open(&made_here, code).await["session"].is_string());
        other.close().await;
        made_here.close().await;
    });
}

#[test]
fn a_server_that_keeps_no_glave_accounts_makes_no_sign_up_links() {
    block_on!(async {
        let dir = folder("signup-links-none");
        let signin = SignIn::local(LocalToken::new([21; 32]), Box::new(fixed_clock()));
        let tested = server(&dir, Vec::new(), Answering::Clients).await;
        let served = serve_with(tested, signin, None).await;
        let refused = link_for(&mut operator(&served)).await;
        assert_eq!(refused, json!(code::NOT_AVAILABLE));
        served.close().await;
    });
}

#[test]
fn someone_signing_up_by_a_link_gives_their_name_first_and_may_then_go_on() {
    block_on!(async {
        let dir = folder("signup-links-named");
        let (served, _) = serve_mailless(&dir).await;
        let code = code_of(&link_for(&mut operator(&served)).await);
        let mut anyone = elsewhere(&served, None);
        let create = Request::PasskeyCreateOptions {
            code,
            link: LinkKind::Signup,
        };
        let options = asked(&mut anyone, &create).await;
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        let mut made = |person_name: Option<&str>| Request::PasskeyCreate {
            ceremony: options["ceremony"].as_str().unwrap().to_owned(),
            credential: phone.create(&options["options"]).unwrap(),
            name: Some("Phone".into()),
            person_name: person_name.map(str::to_owned),
        };
        let unnamed = made(None);
        assert_eq!(
            asked(&mut anyone, &unnamed).await,
            json!(code::INVALID_PARAMS)
        );
        // The same request, answered again with a name: nothing to begin again.
        let named = made(Some("Ada"));
        assert!(asked(&mut anyone, &named).await["session"].is_string());
        served.close().await;
    });
}

#[test]
fn an_emailed_link_opens_nothing_once_sign_up_is_closed() {
    block_on!(async {
        let (open_dir, closed_dir) = (folder("signup-links-open"), folder("signup-links-closed"));
        let outbox = Arc::new(Outbox::default());
        let log = Shared::default();
        let (open_one, _) = serve_sharing(&open_dir, true, &outbox, log.clone()).await;
        // The same store, served again with sign-up turned off.
        let (closed, _) = serve_sharing(&closed_dir, false, &outbox, log).await;
        let start = Request::SignupStart {
            email: "ada@example.com".into(),
        };
        assert_eq!(
            asked(&mut elsewhere(&open_one, None), &start).await,
            json!({})
        );
        let code = linked(&outbox, 1, "signup").await.unwrap();
        assert_eq!(open(&closed, code.clone()).await, json!(code::NO_INVITE));
        assert!(open(&open_one, code).await["session"].is_string());
        closed.close().await;
        open_one.close().await;
    });
}
