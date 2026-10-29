//! glave accounts for invited people (`specs/model.md`, "Identity"): on a server reached at a
//! public origin with no company login, the local person invites someone, whose link makes a
//! passkey of their own; they sign in with any of their passkeys, never as anyone else, and
//! their organization is shown to them, with their role, once it holds two people. Without an
//! address elsewhere, no one can be invited.

use std::path::Path;
use std::sync::Arc;

use glave_event::{OrgRole, PrincipalId, Secret};
use glave_identity::testing::Authenticator;
use glave_ports::host::PublicOrigin;
use glave_ports::{Directory, OrgId};
use glave_session::Answering;
use glave_session::protocol::{Request, code};
use glave_session::signin::{Accounts, Browsers, LocalToken, SignIn};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::issuer::fixed_clock;
use crate::support::web::{ORIGIN, Served, Web, asked, elsewhere, serve_with};
use crate::support::*;

/// A server reached at [`ORIGIN`] if `reached`, keeping glave accounts there, its programs
/// here signing in with `token`.
async fn serve(dir: &Path, reached: bool, token: &LocalToken) -> Served {
    serve_keeping(dir, reached, token).await.0
}

/// [`serve`], with the directory its accounts' organizations are kept in.
async fn serve_keeping(
    dir: &Path,
    reached: bool,
    token: &LocalToken,
) -> (Served, Arc<dyn Directory>) {
    let store = sqlite(&dir.join("sign-ins.db")).await;
    let directory: Arc<dyn Directory> = Arc::new(store.directory());
    let server = "0192f5c6-0000-7000-8000-0000000005e8".to_owned();
    let origin = PublicOrigin::parse(ORIGIN).unwrap();
    let clock = fixed_clock();
    let browsers = Browsers::open(
        store.log(),
        scope(),
        server.clone(),
        Uuid::from_u128(0x692),
        reached.then_some(&origin),
        Box::new(clock.clone()),
    );
    let mut setup = Setup::asking(Vec::new(), Answering::Clients);
    setup.browsers = Some(Arc::new(browsers.await.unwrap()));
    setup.mail = Some(Arc::new(glave_ports::testing::Outbox::default()));
    if reached {
        let directory = Arc::clone(&directory);
        let accounts = Accounts::new(store.log(), directory, server, &origin, Arc::new(clock));
        setup.accounts = Some(Arc::new(accounts.unwrap()));
        setup.origin = Some(origin);
    }
    let (server, _) = setup.build(dir).await;
    let signin = SignIn::local(token.clone(), Box::new(fixed_clock()));
    (serve_with(server, signin, None).await, directory)
}

/// An invite giving `role`, made by the person on the server's computer: its code.
async fn invite(served: &Served, token: &LocalToken, role: OrgRole) -> Secret {
    let mut here = Web::new(served, Some(token.reveal().to_owned()));
    let invited = asked(&mut here, &Request::InviteCreate { role, email: None }).await;
    Secret::new(invited["code"].as_str().unwrap())
}

/// What making a passkey on `device` with the invite `code` is answered with.
async fn join(served: &Served, code: Secret, device: &mut Authenticator) -> Value {
    let mut anyone = elsewhere(served, None);
    let create = Request::PasskeyCreateOptions { code, invite: true };
    let options = asked(&mut anyone, &create).await;
    if !options.is_object() {
        return options;
    }
    let made = Request::PasskeyCreate {
        ceremony: options["ceremony"].as_str().unwrap().to_owned(),
        credential: device.create(&options["options"]).unwrap(),
        name: Some("Phone".into()),
    };
    asked(&mut anyone, &made).await
}

/// A browser elsewhere signed in with `device`'s passkey: its session's token.
async fn sign_in(served: &Served, device: &mut Authenticator) -> Value {
    let mut anyone = elsewhere(served, None);
    let options = asked(&mut anyone, &Request::PasskeyGetOptions {}).await;
    let get = Request::PasskeyGet {
        ceremony: options["ceremony"].as_str().unwrap().to_owned(),
        credential: device.get(&options["options"]).unwrap(),
    };
    asked(&mut anyone, &get).await["session"].clone()
}

/// The first of a `people/list` answer's people who is not the person on the server's
/// computer.
fn invited(people: &Value) -> &Value {
    let listed = people["people"].as_array().unwrap();
    let theirs = listed.iter().find(|person| person.get("local").is_none());
    theirs.unwrap()
}

fn token_of(signed_in: &Value) -> Option<String> {
    signed_in["session"].as_str().map(str::to_owned)
}

#[test]
fn an_invite_makes_a_passkey_whose_person_joins_with_its_role_once() {
    block_on!(async {
        let dir = folder("accounts-join");
        let token = LocalToken::new([11; 32]);
        let served = serve(&dir, true, &token).await;
        let mut anyone = elsewhere(&served, None);
        let before = asked(&mut anyone, &Request::Discover {}).await;
        assert!(before.get("exists").is_none(), "one person: {before}");

        let code = invite(&served, &token, OrgRole::Admin).await;
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        let joined = join(&served, code.clone(), &mut phone).await;
        let mut grace = elsewhere(&served, token_of(&joined));
        let told = asked(&mut grace, &Request::Discover {}).await;
        assert_eq!(told["exists"]["organization"], json!(true), "{told}");
        assert_eq!(told["role"], json!("admin"));
        let people = asked(
            &mut grace,
            &Request::PeopleList {
                after: None,
                limit: None,
            },
        )
        .await;
        let theirs = invited(&people);
        assert_eq!(theirs["role"], json!("admin"), "{people}");
        assert_eq!(theirs["you"], json!(true), "{people}");
        assert!(theirs.get("subject").is_none());
        let start = json!({"method": "thread/start", "params": {}});
        let start: Request = serde_json::from_value(start).unwrap();
        let started = asked(&mut grace, &start).await;
        assert!(started["thread"]["id"].is_string(), "{started}");

        // Spent: the code makes no second account.
        let mut laptop = Authenticator::new(ORIGIN).unwrap();
        let again = join(&served, code, &mut laptop).await;
        assert_eq!(again, json!(code::NO_INVITE));
        let guessed = join(&served, Secret::new("00".repeat(32)), &mut laptop).await;
        assert_eq!(guessed, json!(code::NO_INVITE));
        served.close().await;
    });
}

#[test]
fn people_sign_in_with_any_of_their_passkeys_and_only_as_themselves() {
    block_on!(async {
        let dir = folder("accounts-sign-in");
        let token = LocalToken::new([12; 32]);
        let served = serve(&dir, true, &token).await;
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        let grace = join(
            &served,
            invite(&served, &token, OrgRole::Member).await,
            &mut phone,
        )
        .await;
        let mut tablet = Authenticator::new(ORIGIN).unwrap();
        let linus = join(
            &served,
            invite(&served, &token, OrgRole::Member).await,
            &mut tablet,
        )
        .await;
        let mut grace = elsewhere(&served, token_of(&grace));

        // Grace adds her laptop, and signs in with either.
        let options = asked(&mut grace, &Request::PasskeyAddOptions {}).await;
        let mut laptop = Authenticator::new(ORIGIN).unwrap();
        let added = Request::PasskeyAdd {
            ceremony: options["ceremony"].as_str().unwrap().to_owned(),
            credential: laptop.create(&options["options"]).unwrap(),
            name: Some("Laptop".into()),
        };
        assert!(asked(&mut grace, &added).await["passkey"].is_object());
        let mine = asked(&mut grace, &Request::PasskeyList {}).await;
        assert_eq!(mine["passkeys"].as_array().unwrap().len(), 2, "{mine}");
        let ids = |listed: &Value| -> Vec<Value> {
            let passkeys = listed["passkeys"].as_array().unwrap();
            passkeys
                .iter()
                .map(|passkey| passkey["id"].clone())
                .collect()
        };
        for device in [&mut phone, &mut laptop] {
            let session = sign_in(&served, device).await;
            let mut signed_in = elsewhere(&served, session.as_str().map(str::to_owned));
            let listed = asked(&mut signed_in, &Request::PasskeyList {}).await;
            assert_eq!(
                ids(&listed),
                ids(&mine),
                "the same person, whichever passkey"
            );
        }
        let mut linus = elsewhere(&served, token_of(&linus));
        let theirs = asked(&mut linus, &Request::PasskeyList {}).await;
        assert_eq!(ids(&theirs).len(), 1, "{theirs}");
        assert!(!ids(&mine).contains(&ids(&theirs)[0]), "never another's");

        // Their only passkey stays, as nothing else signs them in yet.
        let only = theirs["passkeys"][0]["id"].as_str().unwrap().to_owned();
        let kept = asked(&mut linus, &Request::PasskeyRemove { passkey: only }).await;
        assert_eq!(kept, json!(code::NOT_ALLOWED));

        // A Member is refused what admins do; signing out ends only their own browser.
        let refused = asked(
            &mut linus,
            &Request::PeopleList {
                after: None,
                limit: None,
            },
        )
        .await;
        assert_eq!(refused, json!(code::NOT_ALLOWED));
        assert_eq!(asked(&mut linus, &Request::SigninEnd {}).await, json!({}));
        let ended = asked(&mut linus, &Request::PasskeyList {}).await;
        assert_eq!(ended, json!(code::NO_SIGN_IN));
        assert!(
            asked(&mut grace, &Request::PasskeyList {})
                .await
                .is_object()
        );
        served.close().await;
    });
}

#[test]
fn the_person_on_the_computer_signs_in_with_their_passkey_once_others_have_accounts() {
    block_on!(async {
        let dir = folder("accounts-owner");
        let token = LocalToken::new([14; 32]);
        let served = serve(&dir, true, &token).await;
        let mut here = Web::new(&served, Some(token.reveal().to_owned()));
        let setup = asked(&mut here, &Request::SigninSetup {}).await;
        let code = Secret::new(setup["code"].as_str().unwrap());
        let mut mac = Authenticator::new(ORIGIN).unwrap();
        let mut anyone = elsewhere(&served, None);
        let create = Request::PasskeyCreateOptions {
            code,
            invite: false,
        };
        let options = asked(&mut anyone, &create).await;
        let made = Request::PasskeyCreate {
            ceremony: options["ceremony"].as_str().unwrap().to_owned(),
            credential: mac.create(&options["options"]).unwrap(),
            name: Some("Mac".into()),
        };
        let theirs = asked(&mut anyone, &made).await;
        let mut owner = elsewhere(&served, token_of(&theirs));
        let own = asked(&mut owner, &Request::PasskeyList {}).await;

        let mut phone = Authenticator::new(ORIGIN).unwrap();
        join(
            &served,
            invite(&served, &token, OrgRole::Member).await,
            &mut phone,
        )
        .await;
        let session = sign_in(&served, &mut mac).await;
        let mut again = elsewhere(&served, session.as_str().map(str::to_owned));
        let listed = asked(&mut again, &Request::PasskeyList {}).await;
        assert_eq!(
            listed["passkeys"][0]["id"], own["passkeys"][0]["id"],
            "{listed}"
        );
        let told = asked(&mut again, &Request::Discover {}).await;
        assert_eq!(told["role"], json!("owner"), "{told}");
        served.close().await;
    });
}

#[test]
fn someone_removed_is_signed_in_to_nothing_here() {
    block_on!(async {
        let dir = folder("accounts-removed");
        let token = LocalToken::new([15; 32]);
        let served = serve(&dir, true, &token).await;
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        let code = invite(&served, &token, OrgRole::Member).await;
        let joined = join(&served, code, &mut phone).await;
        let mut grace = elsewhere(&served, token_of(&joined));
        assert!(
            asked(&mut grace, &Request::PasskeyList {})
                .await
                .is_object()
        );
        let mut here = Web::new(&served, Some(token.reveal().to_owned()));
        let listed = Request::PeopleList {
            after: None,
            limit: None,
        };
        let people = asked(&mut here, &listed).await;
        let person = serde_json::from_value(invited(&people)["id"].clone()).unwrap();
        let removed = asked(&mut here, &Request::PersonRemove { person }).await;
        assert_eq!(removed, json!({}));
        let refused = asked(&mut grace, &Request::PasskeyList {}).await;
        assert_eq!(refused, json!(code::NO_SIGN_IN));
        // With no account left here, no passkey request is for hers.
        let mut anyone = elsewhere(&served, None);
        let options = asked(&mut anyone, &Request::PasskeyGetOptions {}).await;
        assert!(!options.is_object(), "{options}");
        served.close().await;
    });
}

#[test]
fn a_server_no_one_else_reaches_invites_no_one() {
    block_on!(async {
        let dir = folder("accounts-unreached");
        let token = LocalToken::new([13; 32]);
        let served = serve(&dir, false, &token).await;
        let mut here = Web::new(&served, Some(token.reveal().to_owned()));
        let refused = asked(
            &mut here,
            &Request::InviteCreate {
                role: OrgRole::Member,
                email: None,
            },
        )
        .await;
        assert_eq!(refused, json!(code::NOT_ALLOWED));
        served.close().await;
    });
}

#[test]
fn an_account_an_emailed_invite_makes_has_the_address_it_was_sent_to() {
    block_on!(async {
        let dir = folder("accounts-emailed");
        let token = LocalToken::new([17; 32]);
        let served = serve(&dir, true, &token).await;
        let mut here = Web::new(&served, Some(token.reveal().to_owned()));
        let create = Request::InviteCreate {
            role: OrgRole::Member,
            email: Some("grace@example.com".into()),
        };
        let invited = asked(&mut here, &create).await;
        assert_eq!(invited["invite"]["email"], json!("grace@example.com"));
        let code = Secret::new(invited["code"].as_str().unwrap());
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        let joined = join(&served, code, &mut phone).await;
        assert!(token_of(&joined).is_some(), "{joined}");
        let listed = Request::PeopleList {
            after: None,
            limit: None,
        };
        let people = asked(&mut here, &listed).await;
        let emails: Vec<_> = people["people"]
            .as_array()
            .unwrap()
            .iter()
            .map(|person| person["email"].clone())
            .collect();
        assert!(emails.contains(&json!("grace@example.com")), "{people}");
        served.close().await;
    });
}

#[test]
fn an_invites_link_says_what_it_opens_and_someone_signed_in_joins_a_second_organization() {
    block_on!(async {
        let dir = folder("accounts-second");
        let token = LocalToken::new([18; 32]);
        let (served, directory) = serve_keeping(&dir, true, &token).await;
        let mut here = Web::new(&served, Some(token.reveal().to_owned()));
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        let first = invite(&served, &token, OrgRole::Member).await;
        let joined = join(&served, first, &mut phone).await;
        assert!(token_of(&joined).is_some(), "{joined}");
        let listed = Request::PeopleList {
            after: None,
            limit: None,
        };
        let people = asked(&mut here, &listed).await;
        let grace: PrincipalId = serde_json::from_value(invited(&people)["id"].clone()).unwrap();
        // She works in another organization of hers, and is in this one no more.
        let theirs = OrgId::from_uuid(Uuid::from_u128(0x0192_f5c6_0000_7000_8000_0000_0000_0a11));
        directory.leave(grace, scope().org()).await.unwrap();
        directory.join(grace, theirs).await.unwrap();
        let session = sign_in(&served, &mut phone).await;
        let mut signed_in = elsewhere(&served, session.as_str().map(str::to_owned));

        // The link says what it opens to anyone holding it, signed in or not.
        let code = invite(&served, &token, OrgRole::Admin).await;
        let mut anyone = elsewhere(&served, None);
        let show = Request::InviteShow { code: code.clone() };
        let shown = asked(&mut anyone, &show).await;
        let own = scope().org().to_string();
        assert_eq!(shown["shown"]["organization"], json!(own), "{shown}");
        assert_eq!(shown["shown"]["role"], json!("admin"));
        assert_eq!(shown["shown"]["joining"], json!("passkey"));
        assert!(
            shown["shown"].get("by").is_none(),
            "the person here has no name"
        );
        assert!(
            code.expose()
                .starts_with(&scope().org().as_uuid().simple().to_string())
        );

        // Signed in, she joins it as a second organization, and works there by naming it.
        let accepted = asked(
            &mut signed_in,
            &Request::InviteAccept { code: code.clone() },
        )
        .await;
        assert_eq!(accepted["person"]["role"], json!("admin"), "{accepted}");
        assert_eq!(accepted["person"]["you"], json!(true));
        let mut there = elsewhere(&served, session.as_str().map(str::to_owned)).working_in(&own);
        let list: Request =
            serde_json::from_value(json!({"method": "thread/list", "params": {}})).unwrap();
        assert!(asked(&mut there, &list).await["threads"].is_array());
        let orgs = directory.organizations(grace).await.unwrap();
        assert_eq!(orgs, [theirs, scope().org()]);

        // Spent, or to an organization that is none, it opens nothing, and says so alike.
        assert_eq!(asked(&mut anyone, &show).await, json!(code::NO_INVITE));
        let elsewhere_code =
            Secret::new(format!("{}.{}", theirs.as_uuid().simple(), "00".repeat(32)));
        let show = Request::InviteShow {
            code: elsewhere_code,
        };
        assert_eq!(asked(&mut anyone, &show).await, json!(code::NO_INVITE));
        served.close().await;
    });
}

#[test]
fn someone_in_two_organizations_names_the_one_each_request_works_in() {
    block_on!(async {
        let dir = folder("accounts-two-organizations");
        let token = LocalToken::new([16; 32]);
        let (served, directory) = serve_keeping(&dir, true, &token).await;
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        let code = invite(&served, &token, OrgRole::Member).await;
        let joined = join(&served, code, &mut phone).await;
        let mut here = Web::new(&served, Some(token.reveal().to_owned()));
        let listed = Request::PeopleList {
            after: None,
            limit: None,
        };
        let people = asked(&mut here, &listed).await;
        let grace = serde_json::from_value(invited(&people)["id"].clone()).unwrap();
        let other = OrgId::from_uuid(Uuid::from_u128(0x0192_f5c6_0000_7000_8000_0000_0000_0a0c));
        directory.join(grace, other).await.unwrap();

        let start: Request =
            serde_json::from_value(json!({"method": "thread/start", "params": {}})).unwrap();
        let list: Request =
            serde_json::from_value(json!({"method": "thread/list", "params": {}})).unwrap();
        let ids = |listed: &Value| -> Vec<Value> {
            let threads = listed["threads"].as_array().unwrap();
            threads.iter().map(|thread| thread["id"].clone()).collect()
        };
        // Naming none, she works in the first she joined; naming the other, in that one alone.
        let mut first = elsewhere(&served, token_of(&joined));
        let mine = asked(&mut first, &start).await["thread"]["id"].clone();
        let mut there = elsewhere(&served, token_of(&joined)).working_in(&other.to_string());
        let theirs = asked(&mut there, &start).await["thread"]["id"].clone();
        assert!(theirs.is_string(), "{theirs}");
        assert_eq!(ids(&asked(&mut first, &list).await), vec![mine.clone()]);
        assert_eq!(ids(&asked(&mut there, &list).await), vec![theirs]);
        let own = scope().org().to_string();
        let mut named = elsewhere(&served, token_of(&joined)).working_in(&own);
        assert_eq!(ids(&asked(&mut named, &list).await), vec![mine]);

        // One she is not in, one that is no organization's, and a word that is no id are
        // refused alike, before anything is read; the person here is in the server's own alone.
        let unknown = Uuid::from_u128(0x0192_f5c6_0000_7000_8000_0000_0000_0bad).to_string();
        let someone_elses =
            OrgId::from_uuid(Uuid::from_u128(0x0192_f5c6_0000_7000_8000_0000_0000_0e15));
        let someone =
            PrincipalId::try_from(Uuid::from_u128(0x0192_f5c6_0000_7000_8000_0000_0000_0e16))
                .unwrap();
        directory.join(someone, someone_elses).await.unwrap();
        let mut answers = Vec::new();
        for named in [unknown.as_str(), &someone_elses.to_string(), "acme"] {
            let mut refused = elsewhere(&served, token_of(&joined)).working_in(named);
            answers.push(refused.call(&list).await["error"].clone());
        }
        assert_eq!(answers[0]["code"], json!(code::NOT_ALLOWED), "{answers:?}");
        assert!(
            answers.iter().all(|answer| *answer == answers[0]),
            "{answers:?}"
        );
        let mut owner = Web::new(&served, Some(token.reveal().to_owned()));
        let mut other_named = owner_named(&served, &token, &other.to_string());
        assert_eq!(
            asked(&mut other_named, &list).await,
            json!(code::NOT_ALLOWED)
        );
        let mut own_named = owner_named(&served, &token, &own);
        assert!(asked(&mut own_named, &list).await["threads"].is_array());
        assert!(asked(&mut owner, &list).await["threads"].is_array());
        // Without a sign-in, naming one changes nothing of what is open to anyone.
        let mut anyone = elsewhere(&served, None).working_in(&unknown);
        assert!(asked(&mut anyone, &Request::Discover {}).await.is_object());
        served.close().await;
    });
}

/// The person on the server's computer, signed in with `token`, naming `organization`.
fn owner_named(served: &Served, token: &LocalToken, organization: &str) -> Web {
    Web::new(served, Some(token.reveal().to_owned())).working_in(organization)
}

#[test]
fn an_account_not_in_an_organization_joins_it_with_the_address_its_invite_went_to() {
    block_on!(async {
        let dir = folder("accounts-joins-anew");
        let token = LocalToken::new([19; 32]);
        let (served, directory) = serve_keeping(&dir, true, &token).await;
        let mut here = Web::new(&served, Some(token.reveal().to_owned()));
        let mut tablet = Authenticator::new(ORIGIN).unwrap();
        let linus = join(
            &served,
            invite(&served, &token, OrgRole::Member).await,
            &mut tablet,
        )
        .await;
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        let first = Request::InviteCreate {
            role: OrgRole::Member,
            email: Some("grace@first.example.com".into()),
        };
        let first = asked(&mut here, &first).await;
        let first = Secret::new(first["code"].as_str().unwrap());
        join(&served, first, &mut phone).await;
        let listed = Request::PeopleList {
            after: None,
            limit: None,
        };
        let before = asked(&mut here, &listed).await;
        let grace_id: PrincipalId = before["people"]
            .as_array()
            .unwrap()
            .iter()
            .find(|person| person["email"] == json!("grace@first.example.com"))
            .map(|person| serde_json::from_value(person["id"].clone()).unwrap())
            .unwrap();
        // Taken out of this organization, she works in one of her own.
        let removed = asked(&mut here, &Request::PersonRemove { person: grace_id }).await;
        assert_eq!(removed, json!({}));
        let hers = OrgId::from_uuid(Uuid::from_u128(0x0192_f5c6_0000_7000_8000_0000_0000_0a12));
        directory.join(grace_id, hers).await.unwrap();
        let session = sign_in(&served, &mut phone).await;
        let mut signed_in = elsewhere(&served, session.as_str().map(str::to_owned));

        let create = Request::InviteCreate {
            role: OrgRole::Member,
            email: Some("grace@example.com".into()),
        };
        let invited = asked(&mut here, &create).await;
        let code = Secret::new(invited["code"].as_str().unwrap());
        let show = Request::InviteShow { code: code.clone() };
        assert_eq!(
            asked(&mut signed_in, &show).await["shown"]["joining"],
            json!("accept")
        );
        let mut as_linus = elsewhere(&served, token_of(&linus));
        assert_eq!(
            asked(&mut as_linus, &show).await["shown"]["joining"],
            json!("member")
        );
        assert_eq!(
            asked(&mut here, &show).await["shown"]["joining"],
            json!("member")
        );

        let accepted = asked(&mut signed_in, &Request::InviteAccept { code }).await;
        assert_eq!(accepted["person"]["you"], json!(true), "{accepted}");
        let after = asked(&mut here, &listed).await;
        let rejoined = after["people"]
            .as_array()
            .unwrap()
            .iter()
            .find(|person| person["id"] == json!(grace_id))
            .unwrap();
        assert_eq!(rejoined["email"], json!("grace@example.com"), "{after}");
        let orgs = directory.organizations(grace_id).await.unwrap();
        assert_eq!(orgs, [hers, scope().org()]);
        served.close().await;
    });
}
