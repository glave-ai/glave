//! Where a connector is used, through the session API: everywhere, or only in one place of the
//! caller's, a project or their Chats; and a service's second account, named, used in a place of
//! its own, where it takes the place of the one used everywhere, so a place never has two.

use glave_ports::ProjectId;
use glave_session::protocol::{Request, code};
use glave_session::signin::{LocalToken, SignIn};
use serde_json::json;

use crate::connections::{all_connectors, fresh, ids_of};
use crate::support::issuer::fixed_clock;
use crate::support::web::{Web, serve_with};
use crate::support::*;

/// `connector/list` of what the threads of `project` use.
fn used_in(project: ProjectId) -> Request {
    Request::ConnectorList {
        company: None,
        category: None,
        tag: None,
        used_in: Some(project),
    }
}

/// `connector/connect` of DeepWiki's `account`, used only in `only`, or everywhere.
fn connect(account: Option<&str>, only: Option<ProjectId>) -> Request {
    Request::ConnectorConnect {
        connector: "deepwiki".into(),
        account: account.map(str::to_owned),
        only,
    }
}

/// `connector/place` of DeepWiki's `account`, to be used only in `only`, or everywhere.
fn place(account: Option<&str>, only: Option<ProjectId>) -> Request {
    Request::ConnectorPlace {
        connector: "deepwiki".into(),
        account: account.map(str::to_owned),
        only,
    }
}

/// A project of the caller's called "Acme Site", and their Chats.
async fn places(web: &mut Web) -> (ProjectId, ProjectId) {
    let made = Request::ProjectCreate {
        name: "Acme Site".into(),
        folder: None,
    };
    let made = web.call(&made).await;
    let acme = serde_json::from_value(made["result"]["project"]["id"].clone()).unwrap();
    let chat = Request::ThreadStart {
        project: None,
        worktree: None,
        background: false,
        computer: None,
        model: None,
        permissions: None,
        effort: None,
    };
    let chat = web.call(&chat).await;
    let chats = serde_json::from_value(chat["result"]["thread"]["project"].clone()).unwrap();
    (acme, chats)
}

#[test]
fn a_connector_is_used_everywhere_or_only_in_one_place_of_the_caller_s() {
    block_on!(async {
        let token = LocalToken::new([7; 32]);
        let signin = SignIn::local(token.clone(), Box::new(fixed_clock()));
        let dir = folder("conn-connector-places");
        let (tested, _) = fresh(&dir).await;
        let served = serve_with(tested, signin, None).await;
        let mut web = Web::new(&served, Some(token.reveal().to_owned()));
        let (acme, chats) = places(&mut web).await;

        let connected = web.call(&connect(None, Some(acme))).await;
        let account = &connected["result"]["connector"]["accounts"][0];
        assert_eq!(account["used"], "Only in Acme Site", "{connected}");
        assert_eq!(account["only"], json!(acme));
        assert!(
            account.get("name").is_none(),
            "a first account needs no name"
        );
        assert_eq!(ids_of(&web.call(&used_in(acme)).await), ["deepwiki"]);
        let none: Vec<&str> = Vec::new();
        let in_chats = web.call(&used_in(chats)).await;
        assert_eq!(ids_of(&in_chats), none, "not in Chats");

        let placed = web.call(&place(None, None)).await;
        let used = &placed["result"]["connector"]["accounts"][0]["used"];
        assert_eq!(used, "Everywhere, including chats", "{placed}");
        assert_eq!(ids_of(&web.call(&used_in(chats)).await), ["deepwiki"]);
        let placed = web.call(&place(None, Some(chats))).await;
        let used = &placed["result"]["connector"]["accounts"][0]["used"];
        assert_eq!(used, "Only in Chats");
        let listed = web.call(&all_connectors()).await;
        let connectors = &listed["result"]["connectors"];
        assert_eq!(connectors[1]["accounts"][0]["used"], "Only in Chats");
        assert!(connectors[0].get("accounts").is_none(), "unconnected");

        let nowhere = ProjectId::from_uuid(uuid::Uuid::from_u128(7));
        let refused = web.call(&place(None, Some(nowhere))).await;
        assert_eq!(refused["error"]["code"], code::NO_PROJECT, "{refused}");
        let unconnected = Request::ConnectorPlace {
            connector: "linear".into(),
            account: None,
            only: None,
        };
        let refused = web.call(&unconnected).await;
        assert_eq!(refused["error"]["code"], code::NO_CREDENTIAL, "{refused}");
        served.close().await;
    });
}

#[test]
fn a_second_account_is_named_and_takes_the_place_of_the_everywhere_one_in_its_own() {
    block_on!(async {
        let token = LocalToken::new([7; 32]);
        let signin = SignIn::local(token.clone(), Box::new(fixed_clock()));
        let dir = folder("conn-connector-accounts");
        let (tested, _) = fresh(&dir).await;
        let served = serve_with(tested, signin, None).await;
        let mut web = Web::new(&served, Some(token.reveal().to_owned()));
        let (acme, chats) = places(&mut web).await;
        let refused = |answer: &serde_json::Value| {
            assert_eq!(answer["error"]["code"], code::INVALID_PARAMS, "{answer}");
            answer["error"]["message"].as_str().unwrap().to_owned()
        };

        web.call(&connect(Some(" Glave "), None)).await;
        let unnamed = refused(&web.call(&connect(None, Some(acme))).await);
        assert!(unnamed.contains("name this account"), "{unnamed}");
        let in_use = refused(&web.call(&connect(Some("Acme"), None)).await);
        assert!(in_use.contains("used everywhere already"), "{in_use}");
        let long = "a".repeat(41);
        let shape = refused(&web.call(&connect(Some(&long), Some(acme))).await);
        assert!(shape.contains("40 characters"), "{shape}");

        let connected = web.call(&connect(Some("Acme"), Some(acme))).await;
        let accounts = &connected["result"]["connector"]["accounts"];
        assert_eq!(
            *accounts,
            json!([
                {"name": "Glave", "used": "Everywhere, including chats", "status": "connected"},
                {"name": "Acme", "only": acme, "used": "Only in Acme Site", "status": "connected"},
            ]),
            "its name trimmed, each account where it is used"
        );

        let here = web.call(&used_in(acme)).await;
        let deepwiki = &here["result"]["connectors"][0];
        assert_eq!(deepwiki["accounts"].as_array().unwrap().len(), 1);
        assert_eq!(deepwiki["accounts"][0]["name"], "Acme", "{here}");
        let replaced = deepwiki["replaced"].as_str().unwrap();
        assert!(replaced.contains("takes its place"), "{replaced}");
        let in_chats = web.call(&used_in(chats)).await;
        let deepwiki = &in_chats["result"]["connectors"][0];
        assert_eq!(deepwiki["accounts"][0]["name"], "Glave", "{in_chats}");
        assert!(deepwiki.get("replaced").is_none());

        let in_use = refused(&web.call(&place(Some("Glave"), Some(acme))).await);
        assert!(in_use.contains("only in Acme Site already"), "{in_use}");
        let disconnect = Request::ConnectorDisconnect {
            connector: "deepwiki".into(),
            account: Some("Acme".into()),
        };
        web.call(&disconnect).await;
        let listed = web.call(&all_connectors()).await;
        let accounts = &listed["result"]["connectors"][1]["accounts"];
        assert_eq!(accounts.as_array().unwrap().len(), 1, "{listed}");
        assert_eq!(accounts[0]["name"], "Glave");
        served.close().await;
    });
}
