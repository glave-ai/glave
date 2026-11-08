//! The subscriptions a push may go to: one of the browsers' own push services, over `https` on
//! 443, with a key and a secret of the right shapes; never another host, a private network or
//! this computer, outside a test's own service.

use glave_push::{PUSH_SERVICES, Pusher, Refused};

use crate::example::{AUTH, P256DH};

#[test]
fn each_browser_s_push_service_is_taken() {
    let pusher = Pusher::new("https://glave.example.com").unwrap();
    for endpoint in [
        "https://fcm.googleapis.com/fcm/send/abc:def",
        "https://updates.push.services.mozilla.com/wpush/v2/gAAA",
        "https://web.push.apple.com/QGx2v",
        "https://wns2-bl2p.notify.windows.com/w/?token=BQYAAA",
    ] {
        assert!(
            pusher.subscription(endpoint, P256DH, AUTH).is_ok(),
            "{endpoint}"
        );
    }
    assert_eq!(
        PUSH_SERVICES.len(),
        4,
        "Google's, Mozilla's, Apple's, Microsoft's"
    );
    // Padded, as some browsers write it.
    let padded = pusher.subscription(
        "https://web.push.apple.com/QGx2v",
        P256DH,
        &format!("{AUTH}=="),
    );
    assert!(padded.is_ok());
}

#[test]
fn an_endpoint_anywhere_else_is_refused_without_being_said() {
    let pusher = Pusher::new("mailto:ops@example.com").unwrap();
    for endpoint in [
        "https://push.example.net/x",
        "https://fcm.googleapis.com:8443/x",
        "http://fcm.googleapis.com/x",
        "https://127.0.0.1/x",
        "https://localhost/x",
        "https://169.254.169.254/latest",
        "http://127.0.0.1:9/x",
        "https://user:pass@fcm.googleapis.com/x",
        "not a url",
    ] {
        let refused = pusher.subscription(endpoint, P256DH, AUTH).unwrap_err();
        assert_eq!(refused, Refused::Endpoint, "{endpoint}");
        assert!(!refused.to_string().contains(endpoint));
    }
}

#[test]
fn keys_of_the_wrong_shape_are_refused() {
    let pusher = Pusher::new("mailto:ops@example.com").unwrap();
    let endpoint = "https://fcm.googleapis.com/fcm/send/x";
    let half: String = P256DH.chars().take(40).collect();
    let short = pusher.subscription(endpoint, &half, AUTH);
    assert_eq!(short.err(), Some(Refused::Key));
    let secret = pusher.subscription(endpoint, P256DH, "AAAA");
    assert_eq!(secret.err(), Some(Refused::Secret));
}

#[test]
fn a_subscription_shows_its_push_service_alone() {
    let pusher = Pusher::new("mailto:ops@example.com").unwrap();
    let endpoint = "https://fcm.googleapis.com/fcm/send/the-browsers-own";
    let shown = format!("{:?}", pusher.subscription(endpoint, P256DH, AUTH).unwrap());
    assert!(shown.contains("fcm.googleapis.com"), "{shown}");
    assert!(!shown.contains("the-browsers-own"), "{shown}");
    assert!(!shown.contains(AUTH), "{shown}");
}

#[test]
fn the_operator_is_named_as_vapid_asks() {
    assert!(Pusher::new("https://glave.example.com").is_ok());
    assert!(Pusher::new("ops@example.com").is_err());
}
