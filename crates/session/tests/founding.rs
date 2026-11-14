//! Creating organizations (`specs/model.md`, "Identity"): anyone signed in with a glave account
//! creates one, its Owner, to invite people to, as many as ten, sign-up open or closed; their
//! personal space, which sign-up made, never takes invites. The person on the server's computer
//! creates none, and the server's own organization keeps inviting as it does.

use std::sync::Arc;

use glave_event::OrgRole;
use glave_ports::Caller;
use glave_ports::testing::Outbox;
use glave_session::protocol::{Reply, Request, code};
use serde_json::{Value, json};

use glave_identity::testing::Authenticator;
use glave_session::protocol::LinkKind;
use glave_session::signin::LocalToken;

use crate::signups::{organized, serve, signed_up};
use crate::support::web::{ORIGIN, Web, asked, elsewhere, make_passkey};
use crate::support::*;

fn create(name: &str) -> Request {
    Request::OrganizationCreate { name: name.into() }
}

/// The threads `web` lists where it works.
async fn threads(web: &mut Web) -> Value {
    let list: Request =
        serde_json::from_value(json!({"method": "thread/list", "params": {}})).unwrap();
    asked(web, &list).await["threads"].clone()
}

#[test]
fn a_personal_space_takes_no_invites_and_an_organization_created_does() {
    block_on!(async {
        let dir = folder("founding-invites");
        let outbox = Arc::new(Outbox::default());
        let served = serve(&dir, true, &outbox).await;
        let mut ada = signed_up(&served, &outbox, "ada@example.com").await;
        let told = asked(&mut ada, &Request::Discover {}).await;
        assert!(told.get("invites").is_none(), "{told}");
        assert_eq!(told["personal"], json!(true), "{told}");
        assert_eq!(told["organizations"], json!(true), "{told}");
        let invite = Request::InviteCreate {
            role: OrgRole::Member,
            email: None,
        };
        let (message, _) = ada.message(&invite);
        let error = &ada.post(&message).await.messages[0]["error"];
        assert_eq!(error["code"], json!(code::NOT_ALLOWED), "{error}");
        assert_eq!(
            error["message"],
            json!("to invite people, create an organization")
        );
        // One she creates is hers to invite people to, by its name, trimmed.
        let made = asked(&mut ada, &create("  Nurfz ")).await;
        assert_eq!(made["organization"]["name"], json!("Nurfz"), "{made}");
        assert_eq!(made["organization"]["role"], json!("owner"));
        let nurfz = made["organization"]["id"].as_str().unwrap().to_owned();
        let listed = asked(&mut ada, &Request::OrganizationList {}).await;
        let names: Vec<_> = listed["organizations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|org| (org["name"].clone(), org.get("own").is_some()))
            .collect();
        assert_eq!(names, [(json!("Personal"), true), (json!("Nurfz"), false)]);
        let mut there = elsewhere(&served, ada.token.clone()).working_in(&nurfz);
        let told = asked(&mut there, &Request::Discover {}).await;
        assert_eq!(told["invites"], json!(true), "{told}");
        assert!(told.get("personal").is_none(), "{told}");
        assert!(
            told.get("exists").is_none(),
            "alone in it, still none: {told}"
        );
        // While her invite waits it is shown, so she can see and cancel it; cancelled, not.
        let made = asked(&mut there, &invite).await;
        let told = asked(&mut there, &Request::Discover {}).await;
        assert_eq!(told["exists"]["organization"], json!(true), "{told}");
        let id = serde_json::from_value(made["invite"]["id"].clone()).unwrap();
        asked(&mut there, &Request::InviteRevoke { invite: id }).await;
        let told = asked(&mut there, &Request::Discover {}).await;
        assert!(told.get("exists").is_none(), "{told}");
        served.close().await;
    });
}

#[test]
fn a_person_creates_at_most_ten_organizations_each_with_a_name() {
    block_on!(async {
        let dir = folder("founding-most");
        let outbox = Arc::new(Outbox::default());
        let served = serve(&dir, true, &outbox).await;
        let mut ada = signed_up(&served, &outbox, "ada@example.com").await;
        for unnamed in ["", "   "] {
            assert_eq!(
                asked(&mut ada, &create(unnamed)).await,
                json!(code::INVALID_PARAMS)
            );
        }
        let long = "n".repeat(81);
        assert_eq!(
            asked(&mut ada, &create(&long)).await,
            json!(code::INVALID_PARAMS)
        );
        // An organization she joined by an invite is not one she created.
        let mut bob = organized(&served, &outbox, "bob@example.com", "Bob's").await;
        let invite = Request::InviteCreate {
            role: OrgRole::Member,
            email: None,
        };
        let code = asked(&mut bob, &invite).await["code"].clone();
        let accept = Request::InviteAccept {
            code: glave_event::Secret::new(code.as_str().unwrap()),
        };
        assert!(asked(&mut ada, &accept).await["person"].is_object());
        // Names are for people, not unique.
        for _ in 0..10 {
            let made = asked(&mut ada, &create("Lab")).await;
            assert_eq!(made["organization"]["name"], json!("Lab"), "{made}");
        }
        assert_eq!(
            asked(&mut ada, &create("Eleventh")).await,
            json!(code::LIMIT)
        );
        let listed = asked(&mut ada, &Request::OrganizationList {}).await;
        assert_eq!(
            listed["organizations"].as_array().unwrap().len(),
            12,
            "her own, Bob's and the ten she created: {listed}"
        );
        served.close().await;
    });
}

#[test]
fn a_new_organization_is_apart_from_the_personal_space_and_from_everyone_else() {
    block_on!(async {
        let dir = folder("founding-apart");
        let outbox = Arc::new(Outbox::default());
        let served = serve(&dir, true, &outbox).await;
        let mut ada = signed_up(&served, &outbox, "ada@example.com").await;
        let made = asked(&mut ada, &create("Nurfz")).await;
        let nurfz = made["organization"]["id"].as_str().unwrap().to_owned();
        let mut there = elsewhere(&served, ada.token.clone()).working_in(&nurfz);
        let start: Request =
            serde_json::from_value(json!({"method": "thread/start", "params": {}})).unwrap();
        asked(&mut there, &start).await;
        assert_eq!(threads(&mut there).await.as_array().unwrap().len(), 1);
        assert_eq!(
            threads(&mut ada).await,
            json!([]),
            "her personal space holds none of it"
        );
        // Its people are her alone, its Owner, with what her own knows of her.
        let people = Request::PeopleList {
            after: None,
            limit: None,
        };
        let listed = asked(&mut there, &people).await;
        let only = listed["people"].as_array().unwrap();
        assert_eq!(only.len(), 1, "{listed}");
        assert_eq!(only[0]["role"], json!("owner"));
        assert_eq!(only[0]["email"], json!("ada@example.com"));
        // Someone else naming it is refused as for one that does not exist.
        let bob = signed_up(&served, &outbox, "bob@example.com").await;
        let mut intruder = elsewhere(&served, bob.token.clone()).working_in(&nurfz);
        assert_eq!(asked(&mut intruder, &start).await, json!(code::NOT_IN));
        served.close().await;
    });
}

#[test]
fn the_person_on_the_servers_computer_creates_none_and_its_own_organization_invites() {
    let dir = folder("founding-none");
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.identity_provider = true;
        let (server, _) = setup.build(&dir).await;
        let owner = Caller::default();
        let refused = server
            .request_as(&owner, create("Nurfz"))
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::NOT_AVAILABLE);
        let Reply::Server(info) = server.request(Request::Discover {}).await.unwrap() else {
            panic!("server info");
        };
        assert!(!info.personal && !info.organizations, "{info:?}");
        let invite = Request::InviteCreate {
            role: OrgRole::Member,
            email: None,
        };
        let made = server.request_as(&owner, invite).await.unwrap();
        assert!(matches!(made, Reply::Invited { .. }), "{made:?}");
    });
}

#[test]
fn someone_who_joined_by_an_invite_creates_one_where_sign_up_is_closed() {
    block_on!(async {
        let dir = folder("founding-closed");
        let outbox = Arc::new(Outbox::default());
        let served = serve(&dir, false, &outbox).await;
        // The person on the server's computer invites Bob, who makes a glave account with it.
        let mut owner = Web::new(&served, Some(LocalToken::new([21; 32]).reveal().to_owned()));
        let invite = Request::InviteCreate {
            role: OrgRole::Member,
            email: None,
        };
        let code = asked(&mut owner, &invite).await["code"].clone();
        let code = glave_event::Secret::new(code.as_str().unwrap());
        let made = make_passkey(
            &served,
            code,
            LinkKind::Invite,
            &mut Authenticator::new(ORIGIN).unwrap(),
        )
        .await;
        let mut bob = elsewhere(&served, made["session"].as_str().map(str::to_owned));
        let told = asked(&mut bob, &Request::Discover {}).await;
        assert_eq!(told["organizations"], json!(true), "{told}");
        let created = asked(&mut bob, &create("Bob's")).await;
        assert_eq!(created["organization"]["role"], json!("owner"), "{created}");
        // The person on the computer, who has no glave account, is told nothing of it.
        let told = asked(&mut owner, &Request::Discover {}).await;
        assert!(told.get("organizations").is_none(), "{told}");
        served.close().await;
    });
}
