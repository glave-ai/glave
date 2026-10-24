//! The person's connections on a ledger in memory: each server is sent only its own
//! sign-in's token, a token past its time with nothing to renew it is sent nowhere, and a
//! removed sign-in is sent no more.

use glave_event::{At, Principal, Secret, SignInClient};
use glave_ports::testing::{TempDir, fake_secret};

use super::*;

/// A sign-in to `url` with `token`, expiring at `expires` with nothing to renew it.
fn signed_in(url: &str, token: &str, expires: Option<At>) -> ConnectionConnected {
    ConnectionConnected {
        connection: crate::credentials::new_id().unwrap(),
        url: url.to_owned(),
        token: Secret::new(token),
        expires,
        refresh: None,
        client: SignInClient {
            issuer: None,
            client_id: "client".into(),
            scopes: Vec::new(),
        },
        by: Principal::glave(),
    }
}

#[test]
fn each_server_is_sent_only_its_own_sign_in() {
    let home = TempDir::new("conn-own").unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let ledger = crate::accounts::in_memory().await.unwrap();
        let connected = Connected {
            ledger: ledger.clone(),
            home: home.to_path_buf(),
            flows: Flows::default(),
            renewals: Renewals::default(),
        };
        let (docs, tracker) = (
            "https://docs.example.com/mcp",
            "https://tracker.example.com/mcp",
        );
        let [one, two] = [(); 2].map(|()| fake_secret("mcp_"));
        let lapsed = SystemClock
            .now()
            .checked_sub(jiff::SignedDuration::from_secs(60));
        let gone = "https://gone.example.com/mcp";
        for connection in [
            signed_in(docs, &one, None),
            signed_in(tracker, &two, None),
            signed_in(gone, &fake_secret("mcp_"), lapsed.ok().map(At::from)),
        ] {
            ledger.keep_connection(&connection).await.unwrap();
        }
        let sent = |url: &'static str| {
            let connected = &connected;
            async move {
                connected
                    .token(url)
                    .await
                    .map(|token| token.expose().to_owned())
            }
        };
        assert_eq!(sent(docs).await.as_deref(), Some(one.as_str()));
        assert_eq!(sent(tracker).await.as_deref(), Some(two.as_str()));
        assert_eq!(sent("https://other.example.com/mcp").await, None);
        assert_eq!(sent(gone).await, None, "past its time, sent nowhere");
        let listed = connected.list().await.unwrap();
        let expired = listed.iter().find(|kept| kept.url == gone).unwrap();
        assert_eq!(expired.status, ConnectionStatus::Expired);

        connected.remove(docs.to_owned()).await.unwrap();
        assert_eq!(sent(docs).await, None, "removed, sent no more");
        assert_eq!(sent(tracker).await.as_deref(), Some(two.as_str()));
    });
}
