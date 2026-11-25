//! A request with no sign-in is no one's (`specs/session.md`, "Who is calling"): never the
//! person on the server's computer, so nothing of theirs is read or made for it, and what
//! `server/discover` tells it is what the server is and how to sign in.

use std::net::IpAddr;

use glave_event::{Permissions, Principal, PrincipalId, Secret};
use glave_ports::{Caller, ProjectKind};
use glave_session::Answering;
use glave_session::protocol::{ApiError, LinkKind, Reply, Request, code};
use glave_session::signin::{LocalToken, SignIn};
use serde_json::json;

use crate::support::issuer::fixed_clock;
use crate::support::web::{Web, serve_with};
use crate::support::*;

/// The Chats `person` has in the test server's organization, as its store keeps them.
async fn chats_of(server: &Tested, person: PrincipalId) -> usize {
    let theirs = server
        .projects()
        .list(scope().org(), &Principal::person(person), 10)
        .await
        .unwrap();
    theirs
        .iter()
        .filter(|project| project.kind == ProjectKind::Personal)
        .count()
}

/// On a server whose person runs Full access with no sandbox, someone not signed in is told
/// none of it: no mode to choose, glave's own default, and nothing of the server's sandbox.
/// The person, signed in, is still told theirs.
#[test]
fn with_no_sign_in_discovering_tells_only_what_the_server_is() {
    let dir = folder("no-one-discover");
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.settings.permissions = Permissions::FullAccess;
        let (here, _) = setup.build(&dir).await;
        let token = LocalToken::new([31; 32]);
        let signin = SignIn::local(token.clone(), Box::new(fixed_clock()));
        let served = serve_with(here, signin, None).await;
        let told = Web::new(&served, None).call(&Request::Discover {}).await;
        let told = &told["result"];
        assert_eq!(told["name"], "glave", "what the server is");
        assert_eq!(told["permissions"], "auto", "glave's own default");
        assert_eq!(told["modes"], json!([]), "no mode to choose");
        assert!(told.get("unsandboxed").is_none(), "nothing of its sandbox");
        let theirs = Web::new(&served, Some(token.reveal().to_owned()))
            .call(&Request::Discover {})
            .await;
        let theirs = &theirs["result"];
        assert_eq!(theirs["permissions"], "full-access", "the person's");
        assert!(
            theirs["modes"]
                .as_array()
                .unwrap()
                .contains(&json!("full-access")),
            "the person's modes"
        );
        assert!(theirs.get("unsandboxed").is_some(), "the person's sandbox");
        served.close().await;
    });
}

/// No one, asking for what is the caller's own, is refused as a request with no sign-in is,
/// and nothing is made for anyone: no Chats of theirs, nor of the person on the computer.
#[test]
fn no_one_reads_or_makes_anything_of_anyone_s() {
    let dir = folder("no-one-reads");
    block_on!(async {
        let server = server(&dir, Vec::new(), Answering::NoOne).await;
        let no_one = Caller::no_one(IpAddr::from([203, 0, 113, 9]));
        let start = Request::ThreadStart {
            project: None,
            worktree: None,
            background: false,
            computer: None,
            model: None,
            permissions: None,
            effort: None,
        };
        let list = Request::ThreadList {
            project: None,
            limit: None,
            show: None,
            cursor: None,
        };
        for request in [
            Request::ProjectList { limit: None },
            Request::LanguageGet {},
            Request::GroupingGet {},
            Request::FiltersGet {},
            Request::AppearanceGet {},
            Request::LibraryList {},
            Request::ScheduleList {},
            start,
            list,
        ] {
            let method = request.method();
            let refused = server.request_as(&no_one, request).await;
            assert_eq!(
                refused.err().map(|refused| refused.code),
                Some(code::NO_SIGN_IN),
                "{method}"
            );
        }
        let person = server.person().unwrap();
        assert_eq!(chats_of(&server, person).await, 0, "the person here's");
        assert_eq!(
            chats_of(&server, PrincipalId::no_one()).await,
            0,
            "no one's"
        );
    });
}

/// No one opens an invite, or begins making a passkey with one, more than 20 times a minute
/// from one network, each reading its organization's people: past that they are told to wait,
/// while another network, and the person here, ask on. Beginning to sign in is never counted:
/// the server keeps nothing of it.
#[test]
fn a_network_not_signed_in_opens_at_most_twenty_invites_a_minute() {
    let dir = folder("no-one-counted");
    block_on!(async {
        let server = server(&dir, Vec::new(), Answering::NoOne).await;
        let asking = Caller::no_one(IpAddr::from([203, 0, 113, 9]));
        let limited = |answered: Result<Reply, ApiError>| {
            answered.err().map(|refused| refused.code) == Some(code::LIMIT)
        };
        let show = || Request::InviteShow {
            code: Secret::new("not-an-invite"),
        };
        for n in 0..20 {
            let request = if n % 2 == 0 {
                show()
            } else {
                Request::PasskeyCreateOptions {
                    code: Secret::new("not-an-invite"),
                    link: LinkKind::Invite,
                }
            };
            assert!(!limited(server.request_as(&asking, request).await), "{n}");
        }
        assert!(
            limited(server.request_as(&asking, show()).await),
            "its twenty-first"
        );
        for _ in 0..30 {
            let signing_in = server
                .request_as(&asking, Request::PasskeyGetOptions {})
                .await;
            assert!(!limited(signing_in), "signing in is never counted");
        }
        let other = Caller::no_one(IpAddr::from([198, 51, 100, 4]));
        let elsewhere = server.request_as(&other, show()).await;
        assert!(!limited(elsewhere), "another network's first");
        let here = server.request_as(&Caller::default(), show()).await;
        assert!(!limited(here), "the person here is never counted");
    });
}
