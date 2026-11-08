use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use glave_ports::testing::FakeHttp;
use web_push_native::p256::SecretKey;
use web_push_native::{Auth, decrypt};

use super::*;
use crate::example::{AUTH, MESSAGE, P256DH, PLAINTEXT, PRIVATE};

fn bytes(text: &str) -> Vec<u8> {
    URL_SAFE_NO_PAD.decode(text).unwrap()
}

/// The example browser's private key and secret, which read what is sent to it.
fn browser() -> (SecretKey, Auth) {
    let secret: [u8; 16] = bytes(AUTH).try_into().unwrap();
    (
        SecretKey::from_slice(&bytes(PRIVATE)).unwrap(),
        Auth::from(secret),
    )
}

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

#[test]
fn the_rfc_s_example_reads_as_it_says() {
    let (private, secret) = browser();
    let read = decrypt(bytes(MESSAGE), &private, &secret).unwrap();
    assert_eq!(read, PLAINTEXT);
}

#[test]
fn a_push_reaches_its_service_signed_and_readable_only_by_its_browser() {
    let service = FakeHttp::start(vec![(201, String::new())]).unwrap();
    let pusher = Pusher::reaching_this_computer("mailto:operator@example.com").unwrap();
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
    let authorization = &push.headers["authorization"];
    assert!(authorization.starts_with("vapid t="), "{authorization}");
    assert!(authorization.ends_with(&format!("k={}", key.public())));
    assert!(
        !push
            .bytes
            .windows(PLAINTEXT.len())
            .any(|at| at == PLAINTEXT),
        "the push service never sees what it says"
    );
    let (private, secret) = browser();
    let read = decrypt(push.bytes.clone(), &private, &secret).unwrap();
    assert_eq!(read, PLAINTEXT);
}

#[test]
fn a_service_that_says_the_subscription_is_gone_is_heard() {
    for (status, said) in [(410, Sent::Gone), (404, Sent::Gone), (202, Sent::Taken)] {
        let service = FakeHttp::start(vec![(status, String::new())]).unwrap();
        let pusher = Pusher::reaching_this_computer("https://glave.example.com").unwrap();
        let to = pusher
            .subscription(&format!("{}/x", service.url), P256DH, AUTH)
            .unwrap();
        let key = VapidKey::make().unwrap();
        let sent = glave_ports::block_on!(pusher.send(&key, &to, message(b"hi"), now()));
        assert_eq!(sent.unwrap(), said, "{status}");
    }
    let refused = outcome(StatusCode::FORBIDDEN);
    assert!(matches!(
        refused,
        Err(PushError::Refused(StatusCode::FORBIDDEN))
    ));
    assert!(matches!(
        outcome(StatusCode::TOO_MANY_REQUESTS),
        Err(PushError::Refused(_))
    ));
}

#[test]
fn what_a_push_cannot_carry_is_not_sent() {
    let pusher = Pusher::reaching_this_computer("mailto:a@example.com").unwrap();
    let to = pusher
        .subscription("http://127.0.0.1:9/x", P256DH, AUTH)
        .unwrap();
    let key = VapidKey::make().unwrap();
    let long = vec![b'a'; MOST + 1];
    let sent = glave_ports::block_on!(pusher.send(&key, &to, message(&long), now()));
    assert!(matches!(sent, Err(PushError::TooLong(_))));
}

#[test]
fn the_operator_is_named_as_vapid_asks() {
    assert!(Pusher::new("mailto:ops@example.com").is_ok());
    assert!(Pusher::new("https://glave.example.com").is_ok());
    assert!(matches!(
        Pusher::new("ops@example.com"),
        Err(PushError::Contact(_))
    ));
    let pusher = Pusher::new("mailto:ops@example.com").unwrap();
    assert!(
        pusher
            .subscription("http://127.0.0.1:9/x", P256DH, AUTH)
            .is_err(),
        "one on the internet never reaches this computer"
    );
}
