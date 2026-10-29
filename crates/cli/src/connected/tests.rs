//! The person's connections on a ledger in memory: each server is sent only its own
//! sign-in's token, a token past its time with nothing to renew it is sent nowhere, and a
//! removed sign-in is sent no more. Web search turns on into the person's settings and off out
//! of them, never listed as a connection. A server that asks for more than its sign-in grants
//! is listed as asking until the person signs in there again.

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

/// The person's connections on `ledger`, their settings in `home`, with `search`.
fn connected(ledger: &Ledger, home: &Path, search: Option<WebSearch>) -> Connected {
    Connected {
        ledger: ledger.clone(),
        home: home.to_path_buf(),
        flows: Flows::default(),
        renewals: Renewals::default(),
        search,
        writer: Writer::default(),
        more: MoreAsked::default(),
    }
}

#[test]
fn web_search_turns_on_into_the_person_s_settings_and_off_again_never_a_connection() {
    let home = TempDir::new("conn-search").unwrap();
    let runtime = glave_ports::testing::runtime().unwrap();
    // Never reached: nothing here asks web search's server anything.
    let url = "https://search.example.com/mcp?tools=web_search_exa";
    runtime.block_on(async {
        let ledger = crate::accounts::in_memory().await.unwrap();
        let connected = connected(&ledger, &home, Some(WebSearch::at(Some(url.to_owned()))));
        let state = |on| {
            Some(Searching {
                on,
                name: "search".to_owned(),
            })
        };
        assert_eq!(connected.search().await.unwrap(), state(false));
        assert!(
            !home.join(crate::mcp::FILE).exists(),
            "nothing written while off"
        );

        assert_eq!(connected.set_search(true).await.unwrap(), state(true));
        let settings = std::fs::read_to_string(home.join(crate::mcp::FILE)).unwrap();
        let servers = glave_ports::settings::mcp::parse(&settings, &BTreeMap::new()).unwrap();
        assert!(matches!(&servers["search"], ServerConfig::Http { url: at, .. } if at == url));
        assert_eq!(connected.search().await.unwrap(), state(true));
        assert_eq!(
            connected.set_search(true).await.unwrap(),
            state(true),
            "on already"
        );
        assert!(
            connected.list().await.unwrap().is_empty(),
            "never listed as a connection"
        );
        let back = Some("http://127.0.0.1:1/connect/callback".to_owned());
        for named in ["search", url] {
            let refused = connected.start(named.to_owned(), back.clone()).await;
            let refused = refused.unwrap_err();
            assert_eq!(refused.code, code::NO_SERVER, "nor signed in to as one");
        }

        assert_eq!(connected.set_search(false).await.unwrap(), state(false));
        let settings = std::fs::read_to_string(home.join(crate::mcp::FILE)).unwrap();
        assert!(!settings.contains("search.example.com"), "{settings}");
        assert_eq!(
            connected.set_search(false).await.unwrap(),
            state(false),
            "off already"
        );

        // One the person set up there themselves, with a key, stays when it is turned off.
        let keyed = format!(
            r#"{{"mcpServers": {{"exa": {{"url": "{url}", "headers": {{"x-api-key": "k"}}}}}}}}"#
        );
        std::fs::write(home.join(crate::mcp::FILE), &keyed).unwrap();
        let theirs = Some(Searching {
            on: true,
            name: "exa".to_owned(),
        });
        assert_eq!(connected.search().await.unwrap(), theirs);
        let refused = connected.set_search(false).await.unwrap_err();
        assert_eq!(refused.code, code::NOT_AVAILABLE);
        let settings = std::fs::read_to_string(home.join(crate::mcp::FILE)).unwrap();
        assert_eq!(settings, keyed);

        let none = Connected {
            search: None,
            ..connected
        };
        assert_eq!(
            none.search().await.unwrap(),
            None,
            "not the person's to choose"
        );
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
        let connected = connected(&ledger, &home, None);
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

#[test]
fn a_server_that_asks_for_more_is_listed_as_asking_until_signed_in_to_again() {
    let home = TempDir::new("conn-more").unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let ledger = crate::accounts::in_memory().await.unwrap();
        let connected = Arc::new(connected(&ledger, &home, None));
        let (docs, tracker) = (
            "https://docs.example.com/mcp",
            "https://tracker.example.com/mcp",
        );
        for url in [docs, tracker] {
            let connection = signed_in(url, &fake_secret("mcp_"), None);
            ledger.keep_connection(&connection).await.unwrap();
        }
        connected
            .signer(docs)
            .insufficient("files:write files:read");
        connected.signer(docs).insufficient("admin files:write");
        let status = |listed: &[ConnectionInfo], url: &str| {
            listed.iter().find(|kept| kept.url == url).unwrap().status
        };
        let listed = connected.list().await.unwrap();
        assert_eq!(status(&listed, docs), ConnectionStatus::AsksMore);
        assert_eq!(status(&listed, tracker), ConnectionStatus::SignedIn);
        assert_eq!(
            connected.more.of(docs),
            Some(
                ["files:write", "files:read", "admin"]
                    .map(str::to_owned)
                    .to_vec()
            ),
            "everything asked for, for the next sign-in there"
        );

        connected.more.forget(docs);
        let listed = connected.list().await.unwrap();
        assert_eq!(status(&listed, docs), ConnectionStatus::SignedIn);
        connected.signer(tracker).insufficient("admin");
        connected.remove(tracker.to_owned()).await.unwrap();
        assert_eq!(
            connected.more.of(tracker),
            None,
            "forgotten with its sign-in"
        );
    });
}
