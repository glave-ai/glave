//! What `server/discover` says exists beyond a person's own server, each from the server's
//! configuration: nothing on a server that works for one person, and an organization where an
//! identity provider is set up.

use std::sync::Arc;

use glave_event::Principal;
use glave_ports::ProjectKind;
use glave_session::Answering;
use glave_session::protocol::{Exists, Reply, Request, ServerInfo};
use glave_session::signin::{LocalToken, SignIn};

use crate::support::issuer::fixed_clock;
use crate::support::web::{Web, serve_with};
use crate::support::*;

/// What `server` says it is.
async fn discovered(server: &Tested) -> ServerInfo {
    match server.request(Request::Discover {}).await.unwrap() {
        Reply::Server(info) => info,
        other => panic!("server info, not {other:?}"),
    }
}

#[test]
fn a_server_for_one_person_says_nothing_else_exists() {
    let dir = folder("discover-none");
    block_on!(async {
        let server = server(&dir, Vec::new(), Answering::NoOne).await;
        let info = discovered(&server).await;
        assert_eq!(info.exists, Exists::default());
        let sent = serde_json::to_value(&info).unwrap();
        assert!(sent.get("exists").is_none(), "{sent}");
        assert!(sent.get("invite_only").is_none(), "{sent}");
    });
}

#[test]
fn an_identity_provider_makes_an_organization() {
    let dir = folder("discover-organization");
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.identity_provider = true;
        let (server, _) = setup.build(&dir).await;
        let info = discovered(&server).await;
        let organization = Exists {
            organization: true,
            ..Exists::default()
        };
        assert_eq!(info.exists, organization);
        let sent = serde_json::to_value(&info).unwrap();
        assert_eq!(sent["exists"], serde_json::json!({"organization": true}));
    });
}

/// Someone not signed in, discovering the server, reads nothing of the person on its computer:
/// no Chats of theirs is made, where their settings are kept, so none of those is read.
#[test]
fn with_no_sign_in_discovering_makes_and_reads_nothing_of_the_person_here() {
    let dir = folder("discover-unsigned");
    block_on!(async {
        let here = server(&dir, Vec::new(), Answering::NoOne).await;
        let (projects, person) = (Arc::clone(here.projects()), here.person().unwrap());
        let signin = SignIn::local(LocalToken::new([23; 32]), Box::new(fixed_clock()));
        let served = serve_with(here, signin, None).await;
        let told = Web::new(&served, None).call(&Request::Discover {}).await;
        assert_eq!(told["result"]["name"], "glave", "discovered");
        let theirs = projects
            .list(scope().org(), &Principal::person(person), 10)
            .await
            .unwrap();
        let chats = theirs
            .iter()
            .filter(|project| project.kind == ProjectKind::Personal);
        assert_eq!(chats.count(), 0, "no Chats made for the person here");
        served.close().await;
    });
}
