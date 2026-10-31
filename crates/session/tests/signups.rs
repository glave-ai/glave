//! Signing up (`specs/model.md`, "Identity"): where the server's configuration takes sign-ups,
//! keeps glave accounts at a public origin and sends email, someone gives their email, opens
//! the one-time link sent there and makes a passkey, which makes their account and an
//! organization of their own with them its Owner. An address is answered alike whatever it
//! is, a link opens once, one address gets one email a minute, and a server that takes no
//! sign-ups says so.

use std::path::Path;
use std::sync::Arc;

use glave_event::{OrgRole, Secret};
use glave_identity::testing::Authenticator;
use glave_ports::host::PublicOrigin;
use glave_ports::testing::Outbox;
use glave_session::Answering;
use glave_session::protocol::{LinkKind, Request, code};
use glave_session::signin::{Accounts, Browsers, LocalToken, SignIn};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::issuer::fixed_clock;
use crate::support::web::{
    ORIGIN, Served, Web, asked, elsewhere, make_named_passkey, make_passkey, serve_with,
};
use crate::support::*;

/// A server reached at [`ORIGIN`] keeping glave accounts, sending its email to `outbox`, and
/// taking sign-ups if `signup`.
pub(crate) async fn serve(dir: &Path, signup: bool, outbox: &Arc<Outbox>) -> Served {
    serve_timed(dir, signup, outbox).await.0
}

/// [`serve`], with the clock its server reads, to move on.
pub(crate) async fn serve_timed(
    dir: &Path,
    signup: bool,
    outbox: &Arc<Outbox>,
) -> (Served, glave_ports::testing::ManualClock) {
    serve_sharing(dir, signup, outbox, Shared::default()).await
}

/// [`serve_timed`], its server's log `log`, which another server on it shares, as two
/// processes share a database; its glave accounts in `dir` too.
pub(crate) async fn serve_sharing(
    dir: &Path,
    signup: bool,
    outbox: &Arc<Outbox>,
    log: Shared,
) -> (Served, glave_ports::testing::ManualClock) {
    serve_behind(dir, (signup, None), outbox, log).await
}

/// [`serve_sharing`], behind proxies that add `forwarded` addresses to `X-Forwarded-For`
/// where it says, as Google Cloud's load balancer adds two.
pub(crate) async fn serve_behind(
    dir: &Path,
    (signup, forwarded): (bool, Option<std::num::NonZeroU8>),
    outbox: &Arc<Outbox>,
    log: Shared,
) -> (Served, glave_ports::testing::ManualClock) {
    let store = sqlite(&dir.join("sign-ups.db")).await;
    let server = "0192f5c6-0000-7000-8000-0000000005e9".to_owned();
    let origin = PublicOrigin::parse(ORIGIN).unwrap();
    let mut setup = Setup::asking(Vec::new(), Answering::Clients);
    // Passkeys and sessions on the server's own clock, so a test moves them all at once.
    let clock = setup.clock.clone();
    let browsers = Browsers::open(
        store.log(),
        scope(),
        server.clone(),
        Uuid::from_u128(0x693),
        Some(&origin),
        Box::new(clock.clone()),
    );
    setup.browsers = Some(Arc::new(browsers.await.unwrap()));
    setup.mail = Some(Arc::clone(outbox) as Arc<dyn glave_ports::Mailer>);
    let directory = Arc::new(store.directory());
    let accounts = Accounts::new(store.log(), directory, server, &origin, Arc::new(clock));
    setup.accounts = Some(Arc::new(accounts.unwrap()));
    setup.origin = Some(origin);
    setup.signup = signup;
    setup.forwarded = forwarded;
    let ticks = setup.clock.clone();
    let (server, _) = setup.build_on(dir, log).await;
    let signin = SignIn::local(LocalToken::new([21; 32]), Box::new(fixed_clock()));
    (serve_with(server, signin, None).await, ticks)
}

/// The code in the sign-up link of the `count`th message `outbox` was sent.
async fn emailed_code(outbox: &Outbox, count: usize) -> Secret {
    linked(outbox, count, "signup").await.unwrap()
}

/// The code of the link of the kind `fragment` (`signup`, `recover`) in the `count`th message
/// `outbox` was sent, once it was, if it holds one: a link goes after the answer.
pub(crate) async fn linked(outbox: &Outbox, count: usize, fragment: &str) -> Option<Secret> {
    let sent = outbox.sent_by(count).await;
    let text = &sent.get(count.checked_sub(1)?)?.text;
    let marker = format!("#{fragment}=");
    let link = text
        .split_whitespace()
        .find(|word| word.contains(&marker))?;
    Some(Secret::new(link.split_once(&marker)?.1))
}

/// What making a passkey on `device` with the sign-up `code` is answered with.
async fn make(served: &Served, code: Secret, device: &mut Authenticator) -> Value {
    make_passkey(served, code, LinkKind::Signup, device).await
}

#[test]
fn signing_up_makes_an_account_and_an_organization_of_ones_own() {
    block_on!(async {
        let dir = folder("signups-made");
        let outbox = Arc::new(Outbox::default());
        let served = serve(&dir, true, &outbox).await;
        let mut anyone = elsewhere(&served, None);
        let told = asked(&mut anyone, &Request::Discover {}).await;
        assert!(
            told["signin"]
                .as_array()
                .unwrap()
                .contains(&json!("signup")),
            "{told}"
        );

        let start = Request::SignupStart {
            email: " Ada@Example.com ".into(),
        };
        assert_eq!(asked(&mut anyone, &start).await, json!({}));
        let sent = outbox.sent_by(1).await;
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sent[0].to, "ada@example.com",
            "the address as glave reads it"
        );
        assert!(
            sent[0].text.contains(&format!("{ORIGIN}/#signup=")),
            "{}",
            sent[0].text
        );
        // Asked again at once, it is answered alike, and nothing more is sent.
        assert_eq!(asked(&mut anyone, &start).await, json!({}));
        assert_eq!(outbox.sent().len(), 1, "one email a minute to an address");

        let code = emailed_code(&outbox, 1).await;
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        let made = make(&served, code.clone(), &mut phone).await;
        let token = made["session"].as_str().map(str::to_owned);
        assert!(token.is_some(), "{made}");
        let mut ada = elsewhere(&served, token);
        // Her organization is her own, she its one person and Owner.
        let listed = asked(&mut ada, &Request::OrganizationList {}).await;
        let orgs = listed["organizations"].as_array().unwrap();
        assert_eq!(orgs.len(), 1, "{listed}");
        assert_eq!(orgs[0]["own"], json!(true));
        assert_eq!(orgs[0]["current"], json!(true));
        assert_eq!(orgs[0]["people"], json!(1));
        assert_eq!(orgs[0]["role"], json!("owner"));
        // Alone in it, she is shown no organization, and may invite the first.
        let told = asked(&mut ada, &Request::Discover {}).await;
        assert_eq!(told["invites"], json!(true), "{told}");
        assert!(told.get("exists").is_none(), "{told}");
        // While her invite waits it is shown, so she can see and cancel it; cancelled, not.
        let invite = Request::InviteCreate {
            role: OrgRole::Member,
            email: None,
        };
        let made = asked(&mut ada, &invite).await;
        let told = asked(&mut ada, &Request::Discover {}).await;
        assert_eq!(told["exists"]["organization"], json!(true), "{told}");
        let id = serde_json::from_value(made["invite"]["id"].clone()).unwrap();
        asked(&mut ada, &Request::InviteRevoke { invite: id }).await;
        let told = asked(&mut ada, &Request::Discover {}).await;
        assert!(told.get("exists").is_none(), "{told}");
        let listed = Request::PeopleList {
            after: None,
            limit: None,
        };
        let people = asked(&mut ada, &listed).await;
        let only = people["people"].as_array().unwrap();
        assert_eq!(only.len(), 1, "an organization of her own: {people}");
        assert_eq!(only[0]["role"], json!("owner"));
        assert_eq!(only[0]["you"], json!(true));
        assert_eq!(only[0]["email"], json!("ada@example.com"));
        let start_thread: Request =
            serde_json::from_value(json!({"method": "thread/start", "params": {}})).unwrap();
        assert!(asked(&mut ada, &start_thread).await["thread"]["id"].is_string());

        // The link works once.
        let mut laptop = Authenticator::new(ORIGIN).unwrap();
        let again = make(&served, code, &mut laptop).await;
        assert_eq!(again, json!(code::NO_INVITE));
        served.close().await;
    });
}

/// A browser elsewhere signed in with the account `email` signs up for, by its emailed link.
pub(crate) async fn signed_up(served: &Served, outbox: &Outbox, email: &str) -> Web {
    let mut anyone = elsewhere(served, None);
    let start = Request::SignupStart {
        email: email.into(),
    };
    let before = outbox.sent().len();
    assert_eq!(asked(&mut anyone, &start).await, json!({}));
    let mut device = Authenticator::new(ORIGIN).unwrap();
    let code = emailed_code(outbox, before + 1).await;
    let made = make(served, code, &mut device).await;
    elsewhere(served, made["session"].as_str().map(str::to_owned))
}

/// The name `person` goes by among `listed`'s people, the first who is not the caller's own
/// unless `you`.
fn named(listed: &Value, you: bool) -> Value {
    let people = listed["people"].as_array().unwrap();
    let found = people
        .iter()
        .find(|person| person.get("you").is_some() == you && person.get("local").is_none());
    found.unwrap()["name"].clone()
}

#[test]
fn a_name_given_with_the_first_passkey_goes_with_the_person_and_can_be_changed() {
    block_on!(async {
        let dir = folder("signups-names");
        let outbox = Arc::new(Outbox::default());
        let served = serve(&dir, true, &outbox).await;
        let mut anyone = elsewhere(&served, None);
        let start = Request::SignupStart {
            email: "ada@example.com".into(),
        };
        assert_eq!(asked(&mut anyone, &start).await, json!({}));
        let code = emailed_code(&outbox, 1).await;
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        let signup = LinkKind::Signup;
        let refused =
            make_named_passkey(&served, code.clone(), signup, &mut phone, Some("  ")).await;
        assert_eq!(refused, json!(code::INVALID_PARAMS), "no name in it");
        let made =
            make_named_passkey(&served, code, signup, &mut phone, Some(" Ada Lovelace ")).await;
        let mut ada = elsewhere(&served, made["session"].as_str().map(str::to_owned));
        let listed = Request::PeopleList {
            after: None,
            limit: None,
        };
        let keys = asked(&mut ada, &Request::PasskeyList {}).await;
        assert_eq!(keys["person_name"], json!("Ada Lovelace"), "{keys}");
        assert_eq!(keys["nameable"], json!(true));
        assert_eq!(
            named(&asked(&mut ada, &listed).await, true),
            json!("Ada Lovelace")
        );

        // An invite she makes says who made it, and someone it makes an account for is named.
        let invite = Request::InviteCreate {
            role: OrgRole::Member,
            email: None,
        };
        let invited = asked(&mut ada, &invite).await;
        let code = Secret::new(invited["code"].as_str().unwrap());
        let shown = Request::InviteShow { code: code.clone() };
        assert_eq!(
            asked(&mut anyone, &shown).await["shown"]["by"],
            json!("Ada Lovelace")
        );
        let mut laptop = Authenticator::new(ORIGIN).unwrap();
        let bob =
            make_named_passkey(&served, code, LinkKind::Invite, &mut laptop, Some("Bob")).await;
        assert!(bob["session"].is_string(), "{bob}");
        assert_eq!(named(&asked(&mut ada, &listed).await, false), json!("Bob"));

        // Joining another organization, her name goes with her; changed, it changes in both.
        let mut carol = signed_up(&served, &outbox, "carol@example.com").await;
        let team = asked(&mut carol, &invite).await;
        let accept = Request::InviteAccept {
            code: Secret::new(team["code"].as_str().unwrap()),
        };
        asked(&mut ada, &accept).await;
        assert_eq!(
            named(&asked(&mut carol, &listed).await, false),
            json!("Ada Lovelace")
        );
        let renamed = Request::PersonName {
            name: "Ada King".into(),
        };
        assert_eq!(asked(&mut ada, &renamed).await, json!({}));
        assert_eq!(
            named(&asked(&mut carol, &listed).await, false),
            json!("Ada King")
        );
        assert_eq!(
            asked(&mut ada, &Request::PasskeyList {}).await["person_name"],
            json!("Ada King")
        );
        let too_long = Request::PersonName {
            name: "a".repeat(101),
        };
        assert_eq!(
            asked(&mut ada, &too_long).await,
            json!(code::INVALID_PARAMS)
        );
        served.close().await;
    });
}

#[test]
fn an_invite_shows_its_maker_the_organization_until_it_expires() {
    block_on!(async {
        let dir = folder("signups-invite-waits");
        let outbox = Arc::new(Outbox::default());
        let (served, clock) = serve_timed(&dir, true, &outbox).await;
        let mut ada = signed_up(&served, &outbox, "ada@example.com").await;
        let invite = Request::InviteCreate {
            role: OrgRole::Member,
            email: None,
        };
        assert!(asked(&mut ada, &invite).await["code"].is_string());
        // A minute before its week is out it still works, and its organization shows.
        clock.advance_seconds(7 * 24 * 60 * 60 - 60);
        let told = asked(&mut ada, &Request::Discover {}).await;
        assert_eq!(told["exists"]["organization"], json!(true), "{told}");
        clock.advance_seconds(120);
        let told = asked(&mut ada, &Request::Discover {}).await;
        assert!(
            told.get("exists").is_none(),
            "an expired invite shows nothing: {told}"
        );
        served.close().await;
    });
}

#[test]
fn someone_who_signed_up_and_joined_a_team_lists_their_own_then_the_team_by_its_owner() {
    block_on!(async {
        let dir = folder("signups-team");
        let outbox = Arc::new(Outbox::default());
        let served = serve(&dir, true, &outbox).await;
        let mut ada = signed_up(&served, &outbox, "ada@example.com").await;
        let mut bob = signed_up(&served, &outbox, "bob@example.com").await;
        let invite = Request::InviteCreate {
            role: OrgRole::Member,
            email: None,
        };
        let made = asked(&mut ada, &invite).await;
        let code = Secret::new(made["code"].as_str().unwrap());
        let accepted = asked(&mut bob, &Request::InviteAccept { code }).await;
        assert_eq!(accepted["person"]["role"], json!("member"), "{accepted}");
        // His own first, then Ada's, named by her address while it has no name of its own.
        let listed = asked(&mut bob, &Request::OrganizationList {}).await;
        let seen: Vec<_> = listed["organizations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|org| {
                let own = org.get("own").is_some();
                (
                    org["name"].clone(),
                    own,
                    org["role"].clone(),
                    org["people"].clone(),
                )
            })
            .collect();
        let team = (
            json!("ada@example.com's team"),
            false,
            json!("member"),
            json!(2),
        );
        assert_eq!(
            seen,
            [(json!("Personal"), true, json!("owner"), json!(1)), team]
        );
        served.close().await;
    });
}

#[test]
fn an_address_with_an_account_is_answered_alike_and_told_to_sign_in() {
    block_on!(async {
        let dir = folder("signups-had");
        let outbox = Arc::new(Outbox::default());
        let (served, clock) = serve_timed(&dir, true, &outbox).await;
        let mut anyone = elsewhere(&served, None);
        let start = |email: &str| Request::SignupStart {
            email: email.into(),
        };
        asked(&mut anyone, &start("grace@example.com")).await;
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        let made = make(&served, emailed_code(&outbox, 1).await, &mut phone).await;
        assert!(made["session"].is_string(), "{made}");
        // A name around it is no address.
        for written in ["Grace <grace@example.com>", "<grace@example.com>"] {
            let refused = asked(&mut anyone, &start(written)).await;
            assert_eq!(refused, json!(code::INVALID_PARAMS));
        }
        // Asked again a minute later: the same answer, and a note to sign in, with no link.
        clock.advance_seconds(61);
        assert_eq!(
            asked(&mut anyone, &start("GRACE@example.com")).await,
            json!({})
        );
        let sent = outbox.sent_by(2).await;
        assert_eq!(sent.len(), 2);
        assert!(!sent[1].text.contains("#signup="), "{}", sent[1].text);
        assert!(sent[1].subject.contains("already"), "{}", sent[1].subject);
        served.close().await;
    });
}

#[test]
fn a_link_carries_its_address_and_no_one_can_change_it() {
    block_on!(async {
        let dir = folder("signups-forged");
        let outbox = Arc::new(Outbox::default());
        let served = serve(&dir, true, &outbox).await;
        let mut anyone = elsewhere(&served, None);
        let start = Request::SignupStart {
            email: "ada@example.com".into(),
        };
        asked(&mut anyone, &start).await;
        let code = emailed_code(&outbox, 1).await;
        let (secret, _) = code.expose().split_once('.').unwrap();
        let forged = Secret::new(format!("{secret}.{}", hex_of("bea@example.com")));
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        assert_eq!(
            make(&served, forged, &mut phone).await,
            json!(code::NO_INVITE)
        );
        served.close().await;
    });
}

/// `text` in lowercase hex, as a sign-up's code carries its address.
fn hex_of(text: &str) -> String {
    text.bytes()
        .flat_map(|byte| [byte >> 4, byte & 0xf])
        .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
        .collect()
}

#[test]
fn a_server_that_takes_no_sign_ups_says_so() {
    block_on!(async {
        let dir = folder("signups-off");
        let outbox = Arc::new(Outbox::default());
        let served = serve(&dir, false, &outbox).await;
        let mut anyone = elsewhere(&served, None);
        let told = asked(&mut anyone, &Request::Discover {}).await;
        let ways = told["signin"].as_array().cloned().unwrap_or_default();
        assert!(!ways.contains(&json!("signup")), "{told}");
        let start = Request::SignupStart {
            email: "ada@example.com".into(),
        };
        assert_eq!(asked(&mut anyone, &start).await, json!(code::NOT_AVAILABLE));
        let guessed = Secret::new("00".repeat(32));
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        let made = make(&served, guessed, &mut phone).await;
        assert_eq!(made, json!(code::NO_INVITE));
        assert_eq!(outbox.sent(), Vec::new());
        served.close().await;
    });
}
