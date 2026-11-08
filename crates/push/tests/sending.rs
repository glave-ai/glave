//! Pushes sent to a push service on this computer: what it is sent, encrypted only its browser
//! can read and signed with the server's key, and what its answers mean.

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use glave_ports::testing::FakeHttp;
use glave_push::{Message, PushError, Pusher, Sent, Urgency, VapidKey};
use jiff::Timestamp;
use p256::ecdsa::signature::Verifier as _;
use p256::ecdsa::{Signature, VerifyingKey};

use crate::example::{AUTH, P256DH, PLAINTEXT, bytes, read};

fn now() -> Timestamp {
    Timestamp::from_second(1_800_000_000).unwrap()
}

fn message(text: &[u8]) -> Message<'_> {
    Message {
        text,
        keep: Duration::from_secs(600),
        urgency: Urgency::High,
    }
}

fn pusher() -> Pusher {
    Pusher::reaching_this_computer("mailto:operator@example.com").unwrap()
}

/// The claims of the VAPID token in `authorization`, once its signature checks out with the
/// key it names, which must be `key`'s.
fn claims(authorization: &str, key: &VapidKey) -> serde_json::Value {
    let (token, public) = authorization
        .strip_prefix("vapid t=")
        .and_then(|rest| rest.split_once(", k="))
        .unwrap();
    assert_eq!(public, key.public());
    let parts: Vec<&str> = token.split('.').collect();
    let [head, body, signature] = parts.as_slice() else {
        panic!("a JWT has three parts: {token}");
    };
    let head: serde_json::Value = serde_json::from_slice(&bytes(head)).unwrap();
    assert_eq!(head["alg"], "ES256");
    let verifying = VerifyingKey::from_sec1_bytes(&bytes(public)).unwrap();
    let signature = Signature::from_slice(&bytes(signature)).unwrap();
    let signed = format!("{head}.{body}", head = parts[0], body = parts[1]);
    verifying.verify(signed.as_bytes(), &signature).unwrap();
    serde_json::from_slice(&bytes(body)).unwrap()
}

#[test]
fn a_push_reaches_its_service_signed_and_readable_only_by_its_browser() {
    let service = FakeHttp::start(vec![(201, String::new())]).unwrap();
    let origin = service.url.clone();
    let pusher = pusher();
    let to = pusher
        .subscription(&format!("{}/push/abc", service.url), P256DH, AUTH)
        .unwrap();
    let key = VapidKey::make().unwrap();
    let sent = glave_ports::block_on!(pusher.send(&key, &to, message(PLAINTEXT), now()));
    assert_eq!(sent.unwrap(), Sent::Taken);

    let received = service.received();
    let [push] = received.as_slice() else {
        panic!("one push: {received:?}");
    };
    assert_eq!(
        (push.method.as_str(), push.path.as_str()),
        ("POST", "/push/abc")
    );
    assert_eq!(push.headers["content-encoding"], "aes128gcm");
    assert_eq!(push.headers["ttl"], "600");
    assert_eq!(push.headers["urgency"], "high");
    let claims = claims(&push.headers["authorization"], &key);
    assert_eq!(claims["aud"], origin);
    assert_eq!(claims["sub"], "mailto:operator@example.com");
    assert_eq!(claims["exp"], 1_800_000_000 + 12 * 60 * 60);
    assert!(
        !push
            .bytes
            .windows(PLAINTEXT.len())
            .any(|at| at == PLAINTEXT),
        "the push service never sees what it says"
    );
    assert_eq!(read(push.bytes.clone()), PLAINTEXT);
}

#[test]
fn a_service_that_says_the_subscription_is_gone_is_heard_and_one_that_refuses_is_an_error() {
    for (status, said) in [(410, Sent::Gone), (404, Sent::Gone), (202, Sent::Taken)] {
        let service = FakeHttp::start(vec![(status, String::new())]).unwrap();
        let pusher = pusher();
        let to = pusher
            .subscription(&format!("{}/x", service.url), P256DH, AUTH)
            .unwrap();
        let key = VapidKey::make().unwrap();
        let sent = glave_ports::block_on!(pusher.send(&key, &to, message(b"hi"), now()));
        assert_eq!(sent.unwrap(), said, "{status}");
    }
    let service = FakeHttp::start(vec![(403, String::new())]).unwrap();
    let pusher = pusher();
    let at = format!("{}/x", service.url);
    let to = pusher.subscription(&at, P256DH, AUTH).unwrap();
    let key = VapidKey::make().unwrap();
    let sent = glave_ports::block_on!(pusher.send(&key, &to, message(b"hi"), now()));
    let refused = sent.unwrap_err();
    assert!(matches!(refused, PushError::Refused(_)), "{refused}");
    assert!(!refused.to_string().contains(&at), "never the endpoint");
}

#[test]
fn a_push_service_that_cannot_be_reached_is_said_without_its_endpoint() {
    let pusher = pusher();
    // Nothing listens on the discard port.
    let at = "http://127.0.0.1:9/secret-endpoint";
    let to = pusher.subscription(at, P256DH, AUTH).unwrap();
    let key = VapidKey::make().unwrap();
    let sent = glave_ports::block_on!(pusher.send(&key, &to, message(b"hi"), now()));
    let failed = sent.unwrap_err().to_string();
    assert!(!failed.contains("secret-endpoint"), "{failed}");
}

#[test]
fn what_a_push_cannot_carry_is_not_sent() {
    let pusher = pusher();
    let to = pusher
        .subscription("http://127.0.0.1:9/x", P256DH, AUTH)
        .unwrap();
    let key = VapidKey::make().unwrap();
    let long = vec![b'a'; 3_001];
    let sent = glave_ports::block_on!(pusher.send(&key, &to, message(&long), now()));
    assert!(matches!(sent, Err(PushError::TooLong(_))));
}

#[test]
fn a_kept_key_reads_back_as_the_same_key() {
    let key = VapidKey::make().unwrap();
    let again = VapidKey::read(&key.secret()).unwrap();
    assert_eq!(again.public(), key.public());
    assert_ne!(VapidKey::make().unwrap().public(), key.public());
    // A browser's applicationServerKey: 65 bytes, uncompressed.
    assert_eq!(URL_SAFE_NO_PAD.decode(key.public()).unwrap().len(), 65);
    let unreadable = VapidKey::read(&glave_event::Secret::new("not a key"));
    assert_eq!(unreadable.err(), Some(glave_push::KeyError::Unreadable));
}
