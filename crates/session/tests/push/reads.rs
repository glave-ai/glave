//! `push/get` only reads (`specs/invariants.md`, 5): the key every push is signed with is made
//! as the server starts, and someone with no Chats yet is told they have no browsers, their
//! Chats not made to say so.

use glave_event::Principal;
use glave_ports::ProjectKind;

use super::*;

#[test]
fn asking_about_notifications_makes_nothing() {
    let dir = folder("push-reads");
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.identity_provider = true;
        let (server, store) = pushing_server(setup, &dir).await;
        let own = Scope::of_organization(scope().org());
        let key = store.log().read(&own, Reserved::Push.id(), None, 10).await;
        assert_eq!(key.unwrap().len(), 1, "the key, made as the server started");
        let ada = signed_in("ada");
        assert_eq!(
            devices(&server, &ada, None).await,
            Vec::new(),
            "no browsers"
        );
        let theirs = server
            .projects()
            .list(scope().org(), &Principal::person(id("ada")), 10)
            .await
            .unwrap();
        assert!(
            theirs.iter().all(|it| it.kind != ProjectKind::Personal),
            "no Chats made to say so"
        );
        let after = store.log().read(&own, Reserved::Push.id(), None, 10).await;
        assert_eq!(after.unwrap().len(), 1, "no second key");
        server.shutdown().await;
    });
}

/// Asked before the server made its key, `push/get` says it isn't ready rather than make one:
/// the key is the server's to make as it starts.
#[test]
fn asked_before_the_key_is_made_push_get_makes_none() {
    let dir = folder("push-unready");
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.ready_to_push = false;
        let (server, store) = pushing_server(setup, &dir).await;
        let told = server.request(Request::PushGet { endpoint: None }).await;
        let Ok(Reply::Push {
            push: PushInfo::Never { reason },
        }) = told
        else {
            panic!("not ready, not {told:?}");
        };
        assert_eq!(
            &*reason,
            "This server isn't ready to send notifications yet: try again in a moment."
        );
        let own = Scope::of_organization(scope().org());
        let key = store.log().read(&own, Reserved::Push.id(), None, 10).await;
        assert!(key.unwrap().is_empty(), "no key made by a read");
        server.ready_to_push().await.unwrap();
        let (key, _) = offered(
            server
                .request(Request::PushGet { endpoint: None })
                .await
                .unwrap(),
        );
        assert!(!key.is_empty(), "offered once made");
        server.shutdown().await;
    });
}
