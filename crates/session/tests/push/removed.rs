//! Someone taken out of an organization hears nothing more of it (`specs/invariants.md`, 6):
//! no push of a thread there reaches their browsers, which are forgotten there.

use glave_session::protocol::Meta;
use tokio::sync::Notify;

use super::*;

/// Ada's task runs when she is taken out: it ends with her place, and a task's end is always
/// told, but not to her.
#[test]
fn someone_removed_is_pushed_nothing_more_and_their_browsers_there_are_forgotten() {
    let dir = folder("push-removed");
    block_on!(async {
        let (service, mut heard) = push_service();
        let mut setup = Setup::new(vec![answer(Some("Tidied."), None)]);
        setup.identity_provider = true;
        // Her task's model never answers: it runs until she is out.
        setup.gate = Some(Arc::new(Notify::new()));
        let (server, store) = pushing_server(setup, &dir).await;
        let ada = signed_in("ada");
        let phone = Browser::new(12);
        subscribe(&server, &ada, &phone, &format!("{}/ada", service.url)).await;
        let (thread, personal) = chat(&server, &ada).await;
        let task = Request::TurnStart {
            thread,
            prompt: "tidy the notes".into(),
            background: true,
            budget: None,
            meta: Meta::default(),
            model: None,
            permissions: None,
            effort: None,
            unattended: false,
            files: Vec::new(),
        };
        server.request_as(&ada, task).await.unwrap();

        Box::pin(pushing(&server, async {
            let removed = Request::PersonRemove { person: id("ada") };
            server.request(removed).await.unwrap();
            // Rather than tell her how it ended, her browser there is forgotten.
            let forgotten = async {
                while !sealed_names(&store, personal).await.is_empty() {
                    tokio::task::yield_now().await;
                }
            };
            tokio::select! {
                (to, _) = pushed(&mut heard) => panic!("told at {to} once she was out"),
                () = forgotten => {}
            }
        }))
        .await;
        assert!(heard.try_recv().is_err(), "nothing told once she was out");
        server.shutdown().await;
    });
}
