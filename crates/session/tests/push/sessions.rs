//! What keeps a browser the person's: the session it was subscribed in, one browser for each
//! endpoint, its claim erased with it, and one key for every server on the store.

use glave_event::{Auth, AuthMethod, ThreadId};
use glave_session::signin::Browsers;
use sha2::{Digest as _, Sha256};

use super::*;

/// A browser session of the person on this computer, signed in by a link's code.
async fn session(browsers: &Browsers) -> Caller {
    let code = browsers.link().unwrap();
    let session = browsers.redeem(code.expose()).await.unwrap();
    Caller {
        auth: Some(Auth {
            method: AuthMethod::Local,
            issuer: None,
            subject: None,
            session: Some(session.id),
        }),
        ..Caller::default()
    }
}

#[test]
fn a_browser_is_forgotten_once_the_session_it_was_subscribed_in_ends() {
    let dir = folder("push-sessions");
    block_on!(async {
        let (store, pushing) = sealed();
        let mut setup = Setup::new(Vec::new());
        let clock = setup.clock.clone();
        let opened = Browsers::open(
            store.log(),
            scope(),
            "push-sessions".into(),
            uuid::Uuid::from_u128(0x691),
            None,
            Box::new(clock.clone()),
        );
        let browsers = Arc::new(opened.await.unwrap());
        setup.browsers = Some(Arc::clone(&browsers));
        setup.push = Some(pushing);
        setup.projects = Some(Arc::new(store.projects()));
        let server = setup.build_on(&dir, store.log()).await.0;
        let (here, laptop, phone) = (
            session(&browsers).await,
            session(&browsers).await,
            session(&browsers).await,
        );
        let at = |name: &str| format!("https://fcm.googleapis.com/fcm/send/{name}");
        for (caller, n, name) in [
            (&here, 1, "here"),
            (&laptop, 2, "laptop"),
            (&phone, 3, "phone"),
        ] {
            let subscribe = Browser::new(n).subscribe(&at(name), name);
            offered(server.request_as(caller, subscribe).await.unwrap());
        }
        let program = Caller::default();
        assert_eq!(names(&server, &program).await, ["phone", "laptop", "here"]);

        // Signing every other browser out forgets theirs at once.
        let others = Request::SigninEndOthers {};
        server.request_as(&here, others).await.unwrap();
        assert_eq!(names(&server, &program).await, ["here"]);
        // So does signing this one out, and a session past its lifetime.
        let again = session(&browsers).await;
        let subscribe = Browser::new(4).subscribe(&at("again"), "again");
        offered(server.request_as(&again, subscribe).await.unwrap());
        server
            .request_as(&here, Request::SigninEnd {})
            .await
            .unwrap();
        assert_eq!(names(&server, &program).await, ["again"]);
        clock.advance_seconds(31 * 24 * 60 * 60);
        assert_eq!(names(&server, &program).await, Vec::<String>::new());
        server.shutdown().await;
    });
}

#[test]
fn two_tabs_subscribing_one_browser_at_once_keep_it_once() {
    let dir = folder("push-tabs");
    block_on!(async {
        let (server, _store) = pushing_server(Setup::new(Vec::new()), &dir).await;
        let browser = Browser::new(5);
        let at = "https://fcm.googleapis.com/fcm/send/tabs";
        let (first, second) = tokio::join!(
            server.request(browser.subscribe(at, "Chrome on Mac")),
            server.request(browser.subscribe(at, "Chrome on Mac")),
        );
        offered(first.unwrap());
        offered(second.unwrap());
        assert_eq!(names(&server, &Caller::default()).await, ["Chrome on Mac"]);
        server.shutdown().await;
    });
}

/// Whose browser the endpoint `at` is, as its claim's stream holds it, if it still opens.
async fn claim(store: &Store, at: &str) -> Option<serde_json::Value> {
    let digest: [u8; 32] = Sha256::digest(at.as_bytes()).into();
    let stream = ThreadId::new(jiff::Timestamp::UNIX_EPOCH, *digest.first_chunk().unwrap());
    let own = Scope::of_organization(scope().org());
    let read = store
        .log()
        .read(&own, stream.unwrap(), None, 10)
        .await
        .unwrap();
    let opened = read
        .into_iter()
        .filter_map(|raw| match decode(raw).ok()?.body {
            Body::PushClaimed(claimed) => Some(serde_json::to_value(claimed).unwrap()),
            _ => None,
        });
    opened.last()
}

#[test]
fn a_forgotten_browser_s_claim_is_erased_with_it() {
    let dir = folder("push-claim");
    block_on!(async {
        let (server, store) = pushing_server(Setup::new(Vec::new()), &dir).await;
        let at = "https://fcm.googleapis.com/fcm/send/claimed";
        subscribe(&server, &Caller::default(), &Browser::new(6), at).await;
        let listed = devices(&server, &Caller::default(), None).await;
        let claimed = claim(&store, at).await.unwrap();
        assert_eq!(claimed["device"], listed[0].id.to_string());
        let removed = server.request(Request::PushRemove {
            device: listed[0].id,
        });
        offered(removed.await.unwrap());
        assert_eq!(claim(&store, at).await, None, "it no longer opens");
        // Subscribed again, it is claimed again.
        subscribe(&server, &Caller::default(), &Browser::new(6), at).await;
        assert!(claim(&store, at).await.is_some());
        server.shutdown().await;
    });
}

#[test]
fn every_server_on_a_store_signs_with_one_key() {
    let dir = folder("push-one-key");
    block_on!(async {
        let (store, pushing) = sealed();
        let pusher = Pusher::reaching_this_computer("mailto:ops@example.com").unwrap();
        let other = Arc::new(Pushing::new(pusher, store.keyring()));
        let mut servers = Vec::new();
        for (seed, pushing) in [(1, pushing), (2, other)] {
            let mut setup = Setup::new(Vec::new());
            setup.seed = seed;
            setup.push = Some(pushing);
            setup.projects = Some(Arc::new(store.projects()));
            servers.push(setup.build_on(&dir, store.log()).await.0);
        }
        let get = || Request::PushGet { endpoint: None };
        let (one, two) = tokio::join!(servers[0].request(get()), servers[1].request(get()));
        let (one, two) = (offered(one.unwrap()).0, offered(two.unwrap()).0);
        assert_eq!(one, two, "the first key kept is every server's");
        for server in servers {
            server.shutdown().await;
        }
    });
}
