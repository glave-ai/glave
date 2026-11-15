//! Whose computer (`specs/sandbox.md`): someone else's threads on the server's computer get
//! none of its person's git sign-ins (nor secrets, `glave-harness`'s workspace tests): their
//! model is never told the owner's GitHub is signed in, while the owner's is.

use std::sync::Arc;

use glave_event::Forge;
use glave_ports::Caller;
use glave_ports::computer::{CredentialHost, GitCredential};
use glave_ports::testing::{FixedGitCredentials, any_secret};
use glave_session::Answering;
use glave_session::protocol::{Reply, Request};

use crate::support::*;

/// A chat `caller` starts, and a turn in it: whether its model was told `gh` is signed in.
pub(crate) async fn signed_in_for(server: &Tested, seen: &Seen, caller: &Caller) -> bool {
    let started = Request::ThreadStart {
        project: None,
        worktree: None,
        background: false,
        computer: None,
        model: None,
        permissions: None,
        effort: None,
    };
    let Reply::Thread { thread } = server.request_as(caller, started).await.unwrap() else {
        panic!("a thread")
    };
    server
        .request_as(caller, turn_start(thread.id, "hi", None))
        .await
        .unwrap();
    let asked = seen.lock().unwrap();
    let system = &asked.last().unwrap().system;
    serde_json::to_string(system)
        .unwrap()
        .contains("`gh` is signed in for github.com")
}

#[test]
fn someone_else_s_threads_on_the_server_s_computer_get_none_of_its_person_s_sign_ins() {
    let dir = folder("owners-sign-ins");
    block_on!(async {
        let answers = vec![answer(Some("Hi."), None), answer(Some("Hi."), None)];
        let mut setup = Setup::asking(answers, Answering::NoOne);
        setup.identity_provider = true;
        let host = CredentialHost {
            url: "https://github.com".into(),
            forge: Forge::Github,
        };
        let credential = GitCredential::new("x-access-token", any_secret()).unwrap();
        setup.credentials = Some(Arc::new(FixedGitCredentials::new(vec![host], credential)));
        let (server, seen) = setup.build(&dir).await;

        assert!(
            signed_in_for(&server, &seen, &Caller::default()).await,
            "the owner's threads are signed in"
        );
        assert!(
            !signed_in_for(&server, &seen, &signed_in("ada")).await,
            "someone else's get none of the owner's sign-ins"
        );
        server.shutdown().await;
    });
}
