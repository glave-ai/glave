//! The person's connections on a ledger in memory: each server is sent only its own
//! sign-in's token, a token past its time with nothing to renew it is sent nowhere, and a
//! removed sign-in is sent no more. Web search turns on into the person's settings and off out
//! of them, never listed as a connection. A server that asks for more than its sign-in grants
//! is listed as asking until the person signs in there again.

use glave_event::{At, Principal, Secret, SignInClient};
use glave_ports::ProjectId;
use glave_ports::testing::{TempDir, fake_secret};

use super::*;
use crate::accounts::Ledger;

/// A sign-in to `url` with `token`, expiring at `expires` with nothing to renew it.
fn signed_in(url: &str, token: &str, expires: Option<At>) -> ConnectionConnected {
    ConnectionConnected {
        connection: crate::credentials::new_id().unwrap(),
        url: url.to_owned(),
        account: None,
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
        reachable: Reachable::default(),
        offered: Mutex::new(Offered::Every),
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
                    .token(url, None)
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
            .signer(docs, None)
            .insufficient("files:write files:read");
        connected
            .signer(docs, None)
            .insufficient("admin files:write");
        let status = |listed: &[ConnectionInfo], url: &str| {
            listed.iter().find(|kept| kept.url == url).unwrap().status
        };
        let listed = connected.list().await.unwrap();
        assert_eq!(status(&listed, docs), ConnectionStatus::AsksMore);
        assert_eq!(status(&listed, tracker), ConnectionStatus::SignedIn);
        assert_eq!(
            connected.more.of(docs, None),
            Some(
                ["files:write", "files:read", "admin"]
                    .map(str::to_owned)
                    .to_vec()
            ),
            "everything asked for, for the next sign-in there"
        );

        connected.more.forget(docs, None);
        let listed = connected.list().await.unwrap();
        assert_eq!(status(&listed, docs), ConnectionStatus::SignedIn);
        connected.signer(tracker, None).insufficient("admin");
        connected.remove(tracker.to_owned()).await.unwrap();
        assert_eq!(
            connected.more.of(tracker, None),
            None,
            "forgotten with its sign-in"
        );
    });
}

#[test]
fn a_connector_is_connected_listed_and_disconnected_with_its_sign_in() {
    let home = TempDir::new("conn-connectors").unwrap();
    let runtime = glave_ports::testing::runtime().unwrap();
    runtime.block_on(async {
        let ledger = crate::accounts::in_memory().await.unwrap();
        let connected = connected(&ledger, &home, None);
        let status = |listed: &[(KeptConnector, ConnectorStatus)], id: &str| {
            listed.iter().find(|(it, _)| it.id == id).map(|(_, it)| *it)
        };
        let listed = connected.connectors().await.unwrap();
        assert_eq!(
            status(&listed, "deepwiki"),
            Some(ConnectorStatus::Available)
        );
        assert_eq!(status(&listed, "linear"), Some(ConnectorStatus::Available));

        // One that asks no one to sign in is connected at once.
        let done = connected.connect(everywhere("deepwiki"), None).await;
        let done = done.unwrap();
        assert_eq!(done, Connecting::Connected);
        // One that asks for a sign-in has nowhere to come back to without a redirect.
        let nowhere = connected.connect(everywhere("linear"), None).await;
        let nowhere = nowhere.unwrap();
        assert_eq!(nowhere, Connecting::NoWayBack);
        let unknown = connected.connect(everywhere("nothing"), None).await;
        let unknown = unknown.unwrap_err();
        assert_eq!(unknown.code, code::NO_SERVER);

        // A sign-in kept for one, as a finished sign-in keeps it.
        let url = "https://mcp.linear.app/mcp";
        let token = fake_secret("lin_");
        let sign_in = signed_in(url, &token, None);
        ledger
            .keep_connector(&everywhere("linear"), Some(&sign_in))
            .await
            .unwrap();
        let listed = connected.connectors().await.unwrap();
        assert_eq!(
            status(&listed, "deepwiki"),
            Some(ConnectorStatus::Connected)
        );
        assert_eq!(status(&listed, "linear"), Some(ConnectorStatus::Connected));
        let signed = connected.signed().await;
        assert_eq!(
            signed.connectors,
            [everywhere("deepwiki"), everywhere("linear")]
        );
        let connections = connected.list().await.unwrap();
        assert!(
            connections.is_empty(),
            "listed as a connector, not a connection"
        );

        connected.more.ask(url, None, "admin");
        let listed = connected.connectors().await.unwrap();
        assert_eq!(
            status(&listed, "linear"),
            Some(ConnectorStatus::SignInAgain)
        );

        connected.disconnect(everywhere("linear")).await.unwrap();
        let again = connected
            .disconnect(everywhere("linear"))
            .await
            .unwrap_err();
        assert_eq!(again.code, code::NO_CREDENTIAL);
        let accounts = ledger.accounts().await.unwrap();
        assert_eq!(accounts.connectors, [everywhere("deepwiki")]);
        assert!(accounts.connections.is_empty(), "its sign-in went with it");

        // Used only in one project from now on, and only while connected.
        let project = ProjectId::from_uuid(uuid::Uuid::from_u128(7));
        let only = KeptConnector {
            only: Some(project),
            ..everywhere("deepwiki")
        };
        connected.place(only.clone()).await.unwrap();
        assert_eq!(connected.signed().await.connectors, [only]);
        let gone = connected.place(everywhere("linear")).await.unwrap_err();
        assert_eq!(gone.code, code::NO_CREDENTIAL, "not connected");

        connected.offer(Offered::Only(vec!["linear".into()]));
        let listed = connected.connectors().await.unwrap();
        assert_eq!(listed, [(everywhere("linear"), ConnectorStatus::Available)]);
        let refused = connected
            .connect(everywhere("deepwiki"), None)
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::NO_SERVER, "not offered");
    });
}

#[test]
fn two_accounts_of_a_connector_are_signed_in_to_asked_of_and_disconnected_apart() {
    let home = TempDir::new("conn-accounts").unwrap();
    let runtime = glave_ports::testing::runtime().unwrap();
    runtime.block_on(async {
        let ledger = crate::accounts::in_memory().await.unwrap();
        let connected = connected(&ledger, &home, None);
        let url = "https://mcp.linear.app/mcp";
        let (token, theirs) = (fake_secret("lin_"), fake_secret("lin_"));
        let first = signed_in(url, &token, None);
        ledger
            .keep_connector(&everywhere("linear"), Some(&first))
            .await
            .unwrap();
        let acme = KeptConnector {
            account: Some("Acme".into()),
            only: Some(ProjectId::from_uuid(uuid::Uuid::from_u128(8))),
            ..everywhere("linear")
        };
        let second = ConnectionConnected {
            account: acme.account.clone(),
            ..signed_in(url, &theirs, None)
        };
        ledger.keep_connector(&acme, Some(&second)).await.unwrap();
        let statuses = || async {
            let listed = connected.connectors().await.unwrap();
            (listed.into_iter())
                .filter(|(it, _)| it.id == "linear")
                .collect::<Vec<_>>()
        };
        let both = [
            (everywhere("linear"), ConnectorStatus::Connected),
            (acme.clone(), ConnectorStatus::Connected),
        ];
        assert_eq!(statuses().await, both);
        let sent = |account| connected.token(url, account);
        let (one, two) = (sent(None).await.unwrap(), sent(Some("Acme")).await.unwrap());
        assert_eq!(
            (one.expose(), two.expose()),
            (token.as_str(), theirs.as_str())
        );
        let none = connected.list().await.unwrap();
        assert!(none.is_empty(), "a named one is no connection either");

        connected.more.ask(url, Some("Acme"), "admin");
        let asked = statuses().await;
        let asked: Vec<_> = asked.iter().map(|(_, status)| *status).collect();
        assert_eq!(
            asked,
            [ConnectorStatus::Connected, ConnectorStatus::SignInAgain],
            "one account's server asking for more asks nothing of the other"
        );

        let mine = format!(r#"{{"mcpServers": {{"mine": {{"url": "{url}"}}}}}}"#);
        std::fs::write(home.join(crate::mcp::FILE), mine).unwrap();
        let back = Some("http://127.0.0.1:1/connect/callback".to_owned());
        let beta = KeptConnector {
            account: Some("Beta".into()),
            ..acme.clone()
        };
        let refused = connected.connect(beta, back).await.unwrap_err();
        let said = refused.message.to_string();
        assert!(said.contains("your own settings"), "{said}");
        std::fs::remove_file(home.join(crate::mcp::FILE)).unwrap();

        connected.disconnect(acme).await.unwrap();
        let signed = connected.signed().await;
        let left = [(url.to_owned(), None)].into();
        assert_eq!(signed.addresses, left, "the other's sign-in stays");
    });
}

/// The connector `id`, used everywhere.
fn everywhere(id: &str) -> KeptConnector {
    KeptConnector {
        id: id.to_owned(),
        account: None,
        only: None,
    }
}
