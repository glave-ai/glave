//! Push notifications (`specs/session.md`, "Push notifications"): the browsers that take a
//! person's notifications are kept on the server as theirs, listed and forgotten; which kinds
//! they want is theirs to choose; what waits for them, and a task of theirs ending, is pushed to
//! each browser encrypted to its keys; a browser its push service says is gone is forgotten; and
//! a server that pushes nothing says so.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use glave_event::{CallId, Secret, ThreadId};
use glave_ports::testing::FakeHttp;
use glave_ports::{Completion, InMemoryLog};
use glave_push::Pusher;
use glave_session::protocol::{
    Answer, Notification, PushDevice, PushInfo, PushSubscription, Reply, Request, ServerInfo,
    Update, code,
};
use glave_session::{Answering, TurnItem, TurnStream};
use serde_json::json;
use tokio::sync::mpsc;
use web_push_native::p256::SecretKey;
use web_push_native::p256::elliptic_curve::sec1::ToEncodedPoint as _;
use web_push_native::{Auth, decrypt};

use crate::support::*;

/// How long a test waits for a push to reach the push service on this computer.
const PUSHED_WITHIN: Duration = Duration::from_secs(30);

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

/// What pushes, reaching a push service on this computer.
fn pusher() -> Arc<Pusher> {
    Arc::new(Pusher::reaching_this_computer("mailto:ops@example.com").unwrap())
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
fn offered(reply: Reply) -> (String, bool, bool, Vec<PushDevice>) {
    match reply {
        Reply::Push {
            push:
                PushInfo::Offered {
                    key,
                    waiting,
                    finished,
                    devices,
                },
        } => (key, waiting, finished, devices),
        other => panic!("push offered, not {other:?}"),
    }
}

async fn devices(server: &Tested, endpoint: Option<&str>) -> Vec<PushDevice> {
    let get = Request::PushGet {
        endpoint: endpoint.map(str::to_owned),
    };
    offered(server.request(get).await.unwrap()).3
}

#[test]
fn a_person_s_browsers_are_kept_listed_and_forgotten() {
    let dir = folder("push-kept");
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.push = Some(pusher());
        let server = setup.build(&dir).await.0;
        let get = Request::PushGet { endpoint: None };
        let (key, waiting, finished, none) = offered(server.request(get).await.unwrap());
        // A browser subscribes with the server's key: a P-256 point, uncompressed.
        assert_eq!(URL_SAFE_NO_PAD.decode(&key).unwrap().len(), 65);
        assert!(waiting && finished, "every kind until the person chooses");
        assert_eq!(none, Vec::new());

        let (phone, laptop) = (Browser::new(1), Browser::new(2));
        let at_phone = "https://push.example.net/phone";
        let subscribed = server.request(phone.subscribe(at_phone, " Safari on iPhone "));
        let (again, _, _, kept) = offered(subscribed.await.unwrap());
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

        let at_laptop = "https://push.example.net/laptop";
        let both = server.request(laptop.subscribe(at_laptop, "Chrome on Mac"));
        offered(both.await.unwrap());
        let listed = devices(&server, Some(at_phone)).await;
        let names: Vec<(&str, bool)> = listed
            .iter()
            .map(|d| (d.name.as_str(), d.current))
            .collect();
        assert_eq!(
            names,
            [("Chrome on Mac", false), ("Safari on iPhone", true)]
        );

        // The same subscription again is kept once.
        let same = server.request(phone.subscribe(at_phone, "Safari on iPhone"));
        offered(same.await.unwrap());
        let still = devices(&server, None).await;
        assert_eq!(
            still,
            listed
                .iter()
                .map(|d| PushDevice {
                    current: false,
                    ..d.clone()
                })
                .collect::<Vec<_>>()
        );

        // Forgotten, it is told nothing more; forgotten again, nothing changes.
        let phone_id = listed[1].id;
        for _ in 0..2 {
            let removed = server.request(Request::PushRemove { device: phone_id });
            let (_, _, _, left) = offered(removed.await.unwrap());
            let names: Vec<&str> = left.iter().map(|d| d.name.as_str()).collect();
            assert_eq!(names, ["Chrome on Mac"]);
        }
    });
}

#[test]
fn only_a_subscription_a_push_can_reach_is_kept() {
    let dir = folder("push-refused");
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.push = Some(pusher());
        let server = setup.build(&dir).await.0;
        let browser = Browser::new(3);
        for (endpoint, device) in [
            ("http://push.example.net/plain", "Firefox"),
            ("https://10.0.0.1/inside", "Firefox"),
            ("https://push.example.net/x", "   "),
            ("https://push.example.net/x", &"a".repeat(81)),
        ] {
            let refused = server
                .request(browser.subscribe(endpoint, device))
                .await
                .unwrap_err();
            assert_eq!(refused.code, code::INVALID_PARAMS, "{endpoint} {device:?}");
        }
        let broken = Request::PushSubscribe {
            browser: PushSubscription {
                p256dh: "not a key".into(),
                ..browser.subscription("https://push.example.net/x", "Firefox")
            },
        };
        let refused = server.request(broken).await.unwrap_err();
        assert_eq!(refused.code, code::INVALID_PARAMS);
        assert_eq!(devices(&server, None).await, Vec::new());
    });
}

#[test]
fn which_kinds_are_pushed_is_the_person_s_to_choose() {
    let dir = folder("push-kinds");
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.push = Some(pusher());
        let server = setup.build(&dir).await.0;
        let chose = Request::PushSet {
            waiting: false,
            finished: true,
        };
        let (_, waiting, finished, _) = offered(server.request(chose).await.unwrap());
        assert_eq!((waiting, finished), (false, true));
        let get = Request::PushGet { endpoint: None };
        let (_, waiting, finished, _) = offered(server.request(get).await.unwrap());
        assert_eq!((waiting, finished), (false, true), "kept");
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
        let subscribe = Browser::new(4).subscribe("https://push.example.net/x", "Firefox");
        let refused = server.request(subscribe).await.unwrap_err();
        assert_eq!(refused.code, code::NOT_AVAILABLE);
        let chose = Request::PushSet {
            waiting: true,
            finished: true,
        };
        assert_eq!(
            server.request(chose).await.unwrap_err().code,
            code::NOT_AVAILABLE
        );

        let mut pushing = Setup::new(Vec::new());
        pushing.push = Some(pusher());
        let server = pushing.build(&dir).await.0;
        assert!(discovered(server.request(Request::Discover {}).await.unwrap()).push);
    });
}

#[test]
fn what_waits_is_pushed_to_each_browser_and_one_gone_is_forgotten() {
    let dir = folder("push-waiting");
    block_on!(async {
        let (service, mut heard) = push_service();
        let mut setup = Setup::asking(asks(), Answering::Clients);
        setup.push = Some(pusher());
        let server = setup.build(&dir).await.0;
        let (phone, old) = (Browser::new(5), Browser::new(6));
        let at_phone = format!("{}/taken", service.url);
        let subscribed = server.request(phone.subscribe(&at_phone, "Safari on iPhone"));
        subscribed.await.unwrap();
        let at_old = format!("{}/gone", service.url);
        server
            .request(old.subscribe(&at_old, "Old laptop"))
            .await
            .unwrap();
        let thread = start(&server, false).await;

        Box::pin(pushing(&server, async {
            let (mut stream, approval) = waiting(&server, thread).await;
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
            // The browser whose push service said it is gone is forgotten.
            let names: Vec<String> = devices(&server, None)
                .await
                .into_iter()
                .map(|device| device.name)
                .collect();
            assert_eq!(names, ["Safari on iPhone"]);
            allow(&server, thread, approval).await;
            stream.finish().await.unwrap();
        }))
        .await;
        assert!(heard.try_recv().is_err(), "one push for each browser");
    });
}

#[test]
fn a_kind_turned_off_is_not_pushed_and_a_finished_task_is() {
    block_on!(async {
        let (service, mut heard) = push_service();
        let mut answers = asks();
        answers.push(answer(Some("Tidied."), None));
        let mut setup = Setup::asking(answers, Answering::Clients);
        setup.push = Some(pusher());
        let log = InMemoryLog::new();
        let (server, _, _, _dir) = setup.build_in_repository("push-off", log).await;
        let phone = Browser::new(7);
        let at_phone = format!("{}/taken", service.url);
        let subscribed = server.request(phone.subscribe(&at_phone, "Safari on iPhone"));
        subscribed.await.unwrap();
        let chose = Request::PushSet {
            waiting: false,
            finished: true,
        };
        server.request(chose).await.unwrap();
        let thread = start(&server, false).await;

        Box::pin(pushing(&server, async {
            // What waits is not pushed, as the person chose.
            let (mut stream, approval) = waiting(&server, thread).await;
            allow(&server, thread, approval).await;
            stream.finish().await.unwrap();
            // A task's end is, and only it: needs are pushed in turn, so had the wait been
            // pushed, it would have come first.
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

/// A turn whose command waits for the person, then its answer once allowed.
fn asks() -> Vec<Completion> {
    vec![
        answer(None, Some(("bash", json!({"command": "touch ran.txt"})))),
        answer(Some("Done."), None),
    ]
}

/// Runs `body` while `server` pushes what needs people, from before `body` begins.
async fn pushing(server: &Tested, body: impl Future<Output = ()>) {
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

/// A turn in `thread` stopped at the approval its command waits for.
async fn waiting(server: &Tested, thread: ThreadId) -> (TurnStream, CallId) {
    let mut stream = stream_turn(server, thread, "fix the login", None)
        .await
        .unwrap();
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

async fn allow(server: &Tested, thread: ThreadId, approval: CallId) {
    let decide = Request::ApprovalDecide {
        thread,
        approval,
        decision: Answer::AllowOnce,
    };
    server.request(decide).await.unwrap();
}
