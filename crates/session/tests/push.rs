//! Push notifications (`specs/session.md`, "Push notifications"): the browsers that take a
//! person's notifications are kept on the server as theirs, sealed, listed, and erased when
//! forgotten; one browser is only ever one person's; what waits for a person, and a task of
//! theirs ending, is pushed to each of their browsers, and no one else's, encrypted to its keys;
//! a browser its push service says is gone is forgotten; and a server that pushes nothing says
//! so.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use glave_event::{Body, CallId, Principal, PushSubscribed, Secret, ThreadId, decode};
use glave_keys::{InMemoryRoot, KeySettings};
use glave_ports::stream::Reserved;
use glave_ports::testing::FakeHttp;
use glave_ports::{Caller, Completion, EventLog, Projects, Scope};
use glave_push::Pusher;
use glave_session::protocol::{
    Answer, Notification, PushDevice, PushInfo, PushSubscription, Reply, Request, ServerInfo,
    Update, code,
};
use glave_session::{Answering, Pushing, Sent, TurnItem, TurnStream};
use glave_store_sqlite::{SqliteLog, Store};
use serde_json::json;
use tokio::sync::mpsc;
use web_push_native::p256::SecretKey;
use web_push_native::p256::elliptic_curve::sec1::ToEncodedPoint as _;
use web_push_native::{Auth, decrypt};

use crate::support::*;

/// How long a test waits for a push to reach the push service on this computer.
const PUSHED_WITHIN: Duration = Duration::from_secs(30);

/// A server of these tests: on a sealed store.
type Pushed = Tested<SqliteLog>;

/// A browser made for the test: the keys it subscribes with, which read what is pushed to it.
struct Browser {
    secret: SecretKey,
    auth: [u8; 16],
}

impl Browser {
    fn new(seed: u8) -> Self {
        Self {
            secret: SecretKey::from_slice(&[seed; 32]).unwrap(),
            auth: [seed; 16],
        }
    }

    /// Its subscription to the push service at `endpoint`, as `device`.
    fn subscribe(&self, endpoint: &str, device: &str) -> Request {
        Request::PushSubscribe {
            browser: self.subscription(endpoint, device),
        }
    }

    fn subscription(&self, endpoint: &str, device: &str) -> PushSubscription {
        let point = self.secret.public_key().to_encoded_point(false);
        PushSubscription {
            endpoint: endpoint.to_owned(),
            p256dh: URL_SAFE_NO_PAD.encode(point.as_bytes()),
            auth: Secret::new(URL_SAFE_NO_PAD.encode(self.auth)),
            device: device.to_owned(),
        }
    }

    /// What a push to it says.
    fn read(&self, pushed: Vec<u8>) -> serde_json::Value {
        let read = decrypt(pushed, &self.secret, &Auth::from(self.auth)).unwrap();
        serde_json::from_slice(&read).unwrap()
    }
}

/// A sealed store of the test's own, and what pushes, reaching a push service on this computer
/// and erasing forgotten browsers in that store.
fn sealed() -> (Store, Arc<Pushing>) {
    let root = InMemoryRoot::new().unwrap();
    let store = Store::open_in_memory(KeySettings::new(Arc::new(root))).unwrap();
    let pusher = Pusher::reaching_this_computer("mailto:ops@example.com").unwrap();
    let pushing = Arc::new(Pushing::new(pusher, store.keyring()));
    (store, pushing)
}

/// `setup`'s server in `dir`, pushing, on a sealed store of its own, and that store.
async fn pushing_server(mut setup: Setup, dir: &Path) -> (Pushed, Store) {
    let (store, pushing) = sealed();
    setup.push = Some(pushing);
    setup.projects = Some(Arc::new(store.projects()));
    let server = setup.build_on(dir, store.log()).await.0;
    (server, store)
}

/// A push service on this computer that takes every push but to `/gone`, whose subscription is
/// gone, and hands each one's path and body over as it comes.
fn push_service() -> (FakeHttp, mpsc::Receiver<(String, Vec<u8>)>) {
    let (pushed, heard) = mpsc::channel(16);
    let service = FakeHttp::answering_requests(move |request| {
        let _ = pushed.try_send((request.path.clone(), request.bytes.clone()));
        match request.path.as_str() {
            "/gone" => (410, String::new()),
            _ => (201, String::new()),
        }
    })
    .unwrap();
    (service, heard)
}

/// What `push/get`, or a change, says the server offers.
fn offered(reply: Reply) -> (String, Vec<PushDevice>) {
    match reply {
        Reply::Push {
            push: PushInfo::Offered { key, devices },
        } => (key, devices),
        other => panic!("push offered, not {other:?}"),
    }
}

/// `caller`'s browsers, `endpoint`'s marked current.
async fn devices(server: &Pushed, caller: &Caller, endpoint: Option<&str>) -> Vec<PushDevice> {
    let get = Request::PushGet {
        endpoint: endpoint.map(str::to_owned),
    };
    offered(server.request_as(caller, get).await.unwrap()).1
}

/// The names of `caller`'s browsers, the latest first.
async fn names(server: &Pushed, caller: &Caller) -> Vec<String> {
    let listed = devices(server, caller, None).await;
    listed.into_iter().map(|device| device.name).collect()
}

/// Waits until `caller`'s browsers are those named `left`, as the server forgets one.
async fn forgotten(server: &Pushed, caller: &Caller, left: &[&str]) {
    let named = async {
        while names(server, caller).await != left {
            tokio::task::yield_now().await;
        }
    };
    tokio::time::timeout(PUSHED_WITHIN, named).await.unwrap();
}

/// Subscribes `browser` at `endpoint` as `caller`'s `device`.
async fn subscribe(server: &Pushed, caller: &Caller, browser: &Browser, endpoint: &str) {
    let subscribe = browser.subscribe(endpoint, "Safari on iPhone");
    offered(server.request_as(caller, subscribe).await.unwrap());
}

#[test]
fn a_person_s_browsers_are_kept_sealed_listed_and_erased_when_forgotten() {
    let dir = folder("push-kept");
    block_on!(async {
        let (server, store) = pushing_server(Setup::new(Vec::new()), &dir).await;
        let get = Request::PushGet { endpoint: None };
        let (key, none) = offered(server.request(get).await.unwrap());
        // A browser subscribes with the server's key: a P-256 point, uncompressed.
        assert_eq!(URL_SAFE_NO_PAD.decode(&key).unwrap().len(), 65);
        assert_eq!(none, Vec::new());

        let (phone, laptop) = (Browser::new(1), Browser::new(2));
        let at_phone = "https://fcm.googleapis.com/fcm/send/phone";
        let subscribed = server.request(phone.subscribe(at_phone, " Safari on iPhone "));
        let (again, kept) = offered(subscribed.await.unwrap());
        assert_eq!(
            again, key,
            "the key kept is the one every push is signed with"
        );
        let [only] = kept.as_slice() else {
            panic!("one browser: {kept:?}");
        };
        assert_eq!(
            (only.name.as_str(), only.current),
            ("Safari on iPhone", true)
        );

        let at_laptop = "https://updates.push.services.mozilla.com/wpush/v2/laptop";
        let both = server.request(laptop.subscribe(at_laptop, "Firefox on Mac"));
        offered(both.await.unwrap());
        let me = Caller::default();
        let listed = devices(&server, &me, Some(at_phone)).await;
        let marked: Vec<(&str, bool)> = listed
            .iter()
            .map(|d| (d.name.as_str(), d.current))
            .collect();
        assert_eq!(
            marked,
            [("Firefox on Mac", false), ("Safari on iPhone", true)]
        );

        // The same subscription again is kept once.
        let same = server.request(phone.subscribe(at_phone, "Safari on iPhone"));
        offered(same.await.unwrap());
        let unmarked = |device: &PushDevice| PushDevice {
            current: false,
            ..device.clone()
        };
        let still = devices(&server, &me, None).await;
        assert_eq!(still, listed.iter().map(unmarked).collect::<Vec<_>>());

        // Forgotten, it is told nothing more and no longer opens in the store; forgotten again,
        // nothing changes.
        let personal = chat(&server, &me).await.1;
        assert_eq!(sealed_names(&store, personal).await.len(), 2);
        let phone_id = listed[1].id;
        for _ in 0..2 {
            let removed = server.request(Request::PushRemove { device: phone_id });
            let left: Vec<String> = offered(removed.await.unwrap())
                .1
                .into_iter()
                .map(|d| d.name)
                .collect();
            assert_eq!(left, ["Firefox on Mac"]);
        }
        assert_eq!(sealed_names(&store, personal).await, ["Firefox on Mac"]);
        server.shutdown().await;
    });
}

/// The names of the browsers whose subscriptions still open in `personal`'s push ledger.
async fn sealed_names(store: &Store, personal: Scope) -> Vec<String> {
    let events = store
        .log()
        .read(&personal, Reserved::Push.id(), None, 1_000)
        .await
        .unwrap();
    let opened = events
        .into_iter()
        .filter_map(|raw| match decode(raw).ok()?.body {
            Body::PushSubscribed(PushSubscribed { name, .. }) => Some(name),
            _ => None,
        });
    opened.collect()
}

#[test]
fn only_a_subscription_to_a_browser_s_own_push_service_is_kept() {
    let dir = folder("push-refused");
    block_on!(async {
        let (server, _store) = pushing_server(Setup::new(Vec::new()), &dir).await;
        let browser = Browser::new(3);
        let fcm = "https://fcm.googleapis.com/fcm/send/x";
        for (endpoint, device) in [
            ("http://fcm.googleapis.com/fcm/send/x", "Chrome"),
            ("https://fcm.googleapis.com:8443/fcm/send/x", "Chrome"),
            ("https://push.example.net/x", "Chrome"),
            ("https://10.0.0.1/inside", "Chrome"),
            (fcm, "   "),
            (fcm, &"a".repeat(81)),
        ] {
            let refused = server
                .request(browser.subscribe(endpoint, device))
                .await
                .unwrap_err();
            assert_eq!(refused.code, code::INVALID_PARAMS, "{endpoint} {device:?}");
            assert!(!refused.message.contains(endpoint), "{refused}");
        }
        let broken = Request::PushSubscribe {
            browser: PushSubscription {
                p256dh: "not a key".into(),
                ..browser.subscription(fcm, "Chrome")
            },
        };
        let refused = server.request(broken).await.unwrap_err();
        assert_eq!(refused.code, code::INVALID_PARAMS);
        assert_eq!(devices(&server, &Caller::default(), None).await, Vec::new());
        server.shutdown().await;
    });
}

#[test]
fn a_server_that_pushes_nothing_says_so() {
    let dir = folder("push-none");
    block_on!(async {
        let discovered = |reply| match reply {
            Reply::Server(info) => info,
            other => panic!("server info, not {other:?}"),
        };
        let server = Setup::new(Vec::new()).build(&dir).await.0;
        let info: ServerInfo = discovered(server.request(Request::Discover {}).await.unwrap());
        assert!(!info.push);
        let get = Request::PushGet { endpoint: None };
        let Reply::Push {
            push: PushInfo::Never { reason },
        } = server.request(get).await.unwrap()
        else {
            panic!("never pushed");
        };
        assert!(reason.contains("no notifications"), "{reason}");
        let browser = Browser::new(4);
        let subscribe = browser.subscribe("https://fcm.googleapis.com/fcm/send/x", "Chrome");
        let refused = server.request(subscribe).await.unwrap_err();
        assert_eq!(refused.code, code::NOT_AVAILABLE);
        server.shutdown().await;

        let (server, _store) = pushing_server(Setup::new(Vec::new()), &dir).await;
        assert!(discovered(server.request(Request::Discover {}).await.unwrap()).push);
        server.shutdown().await;
    });
}

#[test]
fn what_waits_is_pushed_to_each_browser_and_one_gone_is_forgotten() {
    let dir = folder("push-waiting");
    block_on!(async {
        let (service, mut heard) = push_service();
        let setup = Setup::asking(asks(), Answering::Clients);
        let (server, _store) = pushing_server(setup, &dir).await;
        let me = Caller::default();
        let (phone, old) = (Browser::new(5), Browser::new(6));
        let origin = service.url.clone();
        subscribe(&server, &me, &phone, &format!("{origin}/taken")).await;
        let gone = server.request(old.subscribe(&format!("{origin}/gone"), "Old laptop"));
        offered(gone.await.unwrap());
        let thread = start(&server, false).await;

        Box::pin(pushing(&server, async {
            let (mut stream, approval) = waiting(&server, &me, thread).await;
            let mut paths = Vec::new();
            let mut said = None;
            for _ in 0..2 {
                let (path, body) = pushed(&mut heard).await;
                if path == "/taken" {
                    said = Some(phone.read(body));
                }
                paths.push(path);
            }
            paths.sort();
            assert_eq!(paths, ["/gone", "/taken"]);
            let said = said.unwrap();
            assert_eq!(said["text"], "Needs you: fix the login");
            assert_eq!(said["thread"], thread.to_string());
            assert_eq!(said.as_object().unwrap().len(), 2, "nothing more: {said}");
            // The browser whose push service said it is gone is forgotten, once its answer is in.
            forgotten(&server, &me, &["Safari on iPhone"]).await;
            allow(&server, &me, thread, approval).await;
            stream.finish().await.unwrap();
        }))
        .await;
        assert!(heard.try_recv().is_err(), "one push for each browser");
        server.shutdown().await;
    });
}

#[test]
fn a_task_s_end_is_pushed() {
    block_on!(async {
        let (service, mut heard) = push_service();
        let (store, pushes) = sealed();
        let mut setup = Setup::new(vec![answer(Some("Tidied."), None)]);
        setup.push = Some(pushes);
        let (server, _, _, _dir) = setup.build_in_repository("push-task", store.log()).await;
        let phone = Browser::new(7);
        subscribe(
            &server,
            &Caller::default(),
            &phone,
            &format!("{}/taken", service.url),
        )
        .await;

        Box::pin(pushing(&server, async {
            let task = start_for_tasks(&server, None).await.unwrap();
            background(&server, task, None).await;
            let said = phone.read(pushed(&mut heard).await.1);
            assert_eq!(said["text"], "Done: tidy the notes");
            assert_eq!(said["thread"], task.to_string());
            settle(&server).await;
        }))
        .await;
        assert!(heard.try_recv().is_err(), "nothing else was pushed");
        server.shutdown().await;
    });
}

#[test]
fn what_needs_one_person_is_pushed_to_their_browsers_alone() {
    let dir = folder("push-people");
    block_on!(async {
        let (service, mut heard) = push_service();
        let which = json!({"questions": [{"question": "Which?"}]});
        let asks = || answer(None, Some(("question", which.clone())));
        let mut setup = Setup::new(vec![asks(), asks()]);
        setup.identity_provider = true;
        let (server, _store) = pushing_server(setup, &dir).await;
        let (ada, alan) = (signed_in("ada"), signed_in("alan"));
        let (hers, his) = (Browser::new(8), Browser::new(9));
        subscribe(&server, &ada, &hers, &format!("{}/ada", service.url)).await;
        subscribe(&server, &alan, &his, &format!("{}/alan", service.url)).await;

        Box::pin(pushing(&server, async {
            for (caller, browser, path) in [(&ada, &hers, "/ada"), (&alan, &his, "/alan")] {
                let thread = chat(&server, caller).await.0;
                let started = server.start_streaming(caller, turn_start(thread, "pick one", None));
                let Sent::Started(mut stream) = started.await.unwrap() else {
                    panic!("a turn of its own");
                };
                let (to, body) = pushed(&mut heard).await;
                assert_eq!(to, path, "only the person whose thread it is is told");
                let said = browser.read(body);
                assert_eq!(said["text"], "Needs you: pick one");
                assert_eq!(said["thread"], thread.to_string());
                let cancel = Request::TurnCancel { thread };
                server.request_as(caller, cancel).await.unwrap();
                stream.finish().await.unwrap();
            }
        }))
        .await;
        assert!(heard.try_recv().is_err(), "one push for each need");
        server.shutdown().await;
    });
}

#[test]
fn a_browser_is_whoever_subscribed_it_last_and_reading_makes_nothing() {
    let dir = folder("push-moved");
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.identity_provider = true;
        let (server, store) = pushing_server(setup, &dir).await;
        let (ada, alan, grace) = (signed_in("ada"), signed_in("alan"), signed_in("grace"));
        // Looking makes no project for someone who has none.
        assert_eq!(devices(&server, &grace, None).await, Vec::new());
        let owner = Principal::person(id("grace"));
        let theirs = store
            .projects()
            .list(scope().org(), &owner, 8)
            .await
            .unwrap();
        assert!(theirs.is_empty(), "{theirs:?}");

        let browser = Browser::new(10);
        let shared = "https://web.push.apple.com/QGx-shared";
        subscribe(&server, &ada, &browser, shared).await;
        subscribe(&server, &alan, &browser, shared).await;
        assert_eq!(names(&server, &ada).await, Vec::<String>::new());
        assert_eq!(names(&server, &alan).await, ["Safari on iPhone"]);
        subscribe(&server, &ada, &browser, shared).await;
        assert_eq!(names(&server, &alan).await, Vec::<String>::new());
        assert_eq!(names(&server, &ada).await, ["Safari on iPhone"]);
        server.shutdown().await;
    });
}

/// A new thread in `caller`'s Chats, and where their Chats are.
async fn chat(server: &Pushed, caller: &Caller) -> (ThreadId, Scope) {
    let started = Request::ThreadStart {
        project: None,
        background: false,
        worktree: None,
        computer: None,
        model: None,
        permissions: None,
        effort: None,
    };
    match server.request_as(caller, started).await.unwrap() {
        Reply::Thread { thread } => (thread.id, Scope::glaves_own(scope().org(), thread.project)),
        other => panic!("a thread, not {other:?}"),
    }
}

/// A turn whose command waits for the person, then its answer once allowed.
fn asks() -> Vec<Completion> {
    vec![
        answer(None, Some(("bash", json!({"command": "touch ran.txt"})))),
        answer(Some("Done."), None),
    ]
}

/// Runs `body` while `server` pushes what needs people, from before `body` begins.
async fn pushing(server: &Pushed, body: impl Future<Output = ()>) {
    tokio::select! {
        biased;
        () = server.push_notifications() => unreachable!("it pushes until dropped"),
        () = body => {}
    }
}

/// The next push the push service took: where, and what it carried.
async fn pushed(heard: &mut mpsc::Receiver<(String, Vec<u8>)>) -> (String, Vec<u8>) {
    tokio::time::timeout(PUSHED_WITHIN, heard.recv())
        .await
        .unwrap()
        .unwrap()
}

/// A turn of `caller`'s in `thread`, stopped at the approval its command waits for.
async fn waiting(server: &Pushed, caller: &Caller, thread: ThreadId) -> (TurnStream, CallId) {
    let started = server.start_streaming(caller, turn_start(thread, "fix the login", None));
    let Sent::Started(mut stream) = started.await.unwrap() else {
        panic!("a turn of its own");
    };
    loop {
        match stream.next().await.unwrap() {
            TurnItem::Note(note) => {
                if let Notification::Live {
                    update: Update::ApprovalPending { approval },
                    ..
                } = note.as_ref()
                {
                    let approval = approval.clone();
                    return (stream, approval);
                }
            }
            other => panic!("the turn went on without asking: {other:?}"),
        }
    }
}

async fn allow(server: &Pushed, caller: &Caller, thread: ThreadId, approval: CallId) {
    let decide = Request::ApprovalDecide {
        thread,
        approval,
        decision: Answer::AllowOnce,
    };
    server.request_as(caller, decide).await.unwrap();
}
