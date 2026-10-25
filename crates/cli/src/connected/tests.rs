//! The person's connections on a ledger in memory: each server is sent only its own
//! sign-in's token, a token past its time with nothing to renew it is sent nowhere, and a
//! removed sign-in is sent no more.

use glave_event::{At, Principal, Secret, SignInClient};
use glave_ports::testing::{TempDir, fake_secret};

use super::*;
use crate::accounts::Ledger;

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

/// The person's connections on `ledger`, their settings in `home`, glave suggesting `suggested`.
fn connected(ledger: &Ledger, home: &Path, suggested: Vec<Suggestion>) -> Connected {
    Connected {
        ledger: ledger.clone(),
        home: home.to_path_buf(),
        flows: Flows::default(),
        renewals: Renewals::default(),
        suggested,
        writer: Writer::default(),
    }
}

#[test]
fn a_suggestion_is_off_until_turned_on_into_the_person_s_settings_and_off_again() {
    let home = TempDir::new("conn-suggested").unwrap();
    let runtime = glave_ports::testing::runtime().unwrap();
    // Never reached: a list asks a suggestion's server nothing.
    let url = "https://search.example.com/mcp?tools=web_search_exa";
    runtime.block_on(async {
        let ledger = crate::accounts::in_memory().await.unwrap();
        let search = Suggestion::search(Some(url.to_owned()));
        let connected = connected(&ledger, &home, vec![search]);
        let status = |listed: &[ConnectionInfo]| {
            let found = listed.iter().find(|it| it.suggested.is_some());
            found.map(|it| (it.name.clone(), it.status))
        };
        let off = Some((Some("search".to_owned()), ConnectionStatus::Off));
        assert_eq!(status(&connected.list().await.unwrap()), off);

        let on = connected.add(url.to_owned()).await.unwrap();
        assert_eq!((on.url.as_str(), on.status), (url, ConnectionStatus::On));
        assert_eq!(on.suggested.unwrap().title.to_string(), "Search the web");
        let settings = std::fs::read_to_string(home.join(crate::mcp::FILE)).unwrap();
        let servers = glave_ports::settings::mcp::parse(&settings, &BTreeMap::new()).unwrap();
        assert!(matches!(&servers["search"], ServerConfig::Http { url: at, .. } if at == url));
        let listed = connected.list().await.unwrap();
        let on = Some((Some("search".to_owned()), ConnectionStatus::On));
        assert_eq!(status(&listed), on);
        assert_eq!(listed.len(), 1, "listed once, as the suggestion it is");
        assert_eq!(
            connected.add(url.to_owned()).await.unwrap().status,
            ConnectionStatus::On
        );

        let refused = connected.add("docs".to_owned()).await.unwrap_err();
        assert_eq!(refused.code, code::NO_SERVER);
        connected.remove("search".to_owned()).await.unwrap();
        assert_eq!(status(&connected.list().await.unwrap()), off);
        let settings = std::fs::read_to_string(home.join(crate::mcp::FILE)).unwrap();
        assert!(!settings.contains("search.example.com"), "{settings}");
        let again = connected.remove("search".to_owned()).await.unwrap_err();
        assert_eq!(again.code, code::NO_CREDENTIAL);

        // One the person set up there themselves, with a key, stays when it is turned off.
        let keyed = format!(
            r#"{{"mcpServers": {{"search": {{"url": "{url}", "headers": {{"x-api-key": "k"}}}}}}}}"#
        );
        std::fs::write(home.join(crate::mcp::FILE), &keyed).unwrap();
        let theirs = connected.remove("search".to_owned()).await.unwrap_err();
        assert_eq!(theirs.code, code::NOT_AVAILABLE);
        let settings = std::fs::read_to_string(home.join(crate::mcp::FILE)).unwrap();
        assert_eq!(settings, keyed);
    });
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
        let connected = connected(&ledger, &home, Vec::new());
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
