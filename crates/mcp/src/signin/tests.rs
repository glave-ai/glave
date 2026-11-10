use glave_ports::testing::FakeHttp;

use super::*;

const ON_THE_INTERNET: &str = "https://mcp.example.com/mcp";

/// rmcp's sign-in, on the person's own computer, for the server at `url` whose
/// authorization server says to register and trade for tokens at `endpoint`.
async fn naming(url: &str, endpoint: &str) -> AuthorizationManager {
    let mut manager = guarded(url, Reachable::AlsoPrivate).await.unwrap();
    let metadata = json!({
        "issuer": "https://auth.example.com",
        "authorization_endpoint": "https://auth.example.com/authorize",
        "registration_endpoint": endpoint,
        "token_endpoint": endpoint,
    });
    manager.set_metadata(serde_json::from_value(metadata).unwrap());
    manager
}

/// Whether `error` is glave's client refusing to reach a private address for a server on
/// the internet.
fn kept_off(error: &AuthError) -> bool {
    causes(error).any(|cause| {
        cause
            .to_string()
            .contains("while signing in to a server on the internet")
    })
}

#[test]
fn a_server_on_the_internet_has_glave_register_and_trade_nowhere_private() {
    const REDIRECT: &str = "http://127.0.0.1:1/connect/callback";
    let here = FakeHttp::always(200, "{}").unwrap();
    let port = here.host.rsplit_once(':').unwrap().1.to_owned();
    let on_this_computer = [
        format!("http://127.0.0.1:{port}/oauth"),
        format!("http://localhost:{port}/oauth"),
        format!("http://[::ffff:127.0.0.1]:{port}/oauth"),
    ];
    let on_a_network = ["https://10.0.0.9/oauth", "https://192.168.1.1/oauth"];
    // Trades a code at `endpoint` for a sign-in to the server at `url`, already registered.
    let trade = async |url: &str, endpoint: &str| {
        let request = AuthorizationRequest::new(REDIRECT).with_preregistered_client("glave");
        let session = AuthorizationSession::new(naming(url, endpoint).await, request);
        let session = session.await.map_err(|(_, error)| error).unwrap();
        let page = reqwest::Url::parse(session.get_authorization_url()).unwrap();
        let (_, state) = page
            .query_pairs()
            .find(|(name, _)| name == "state")
            .unwrap();
        session
            .handle_callback_with_issuer("a-code", &state, None)
            .await
    };
    glave_ports::block_on!(async {
        for endpoint in on_this_computer
            .iter()
            .map(String::as_str)
            .chain(on_a_network)
        {
            let request = AuthorizationRequest::new(REDIRECT);
            let manager = naming(ON_THE_INTERNET, endpoint).await;
            let Err((_, error)) = AuthorizationSession::new(manager, request).await else {
                panic!("{endpoint}: it registered");
            };
            assert!(kept_off(&error), "{endpoint}: {error}");
        }
        // rmcp says only that a trade failed, so what this computer received says whether
        // it was sent: never for the server on the internet, and for one on this computer.
        for endpoint in &on_this_computer {
            assert!(
                trade(ON_THE_INTERNET, endpoint).await.is_err(),
                "{endpoint}"
            );
        }
        let mine = format!("http://localhost:{port}/mcp");
        assert!(
            trade(&mine, &on_this_computer[0]).await.is_err(),
            "no token"
        );
    });
    assert_eq!(
        here.received().len(),
        1,
        "reached for the server on it alone"
    );
}

#[test]
fn its_requests_go_out_under_the_rule() {
    let here = FakeHttp::always(404, "{}").unwrap();
    let port = here.host.rsplit_once(':').unwrap().1;
    let url = format!("http://localhost:{port}/mcp");
    glave_ports::block_on!(async {
        let manager = guarded(&url, Reachable::OnlyPublic).await.unwrap();
        let failed = manager.resolve_metadata().await.unwrap_err();
        let said = SignInError::from_auth(&url, &failed).to_string();
        assert!(
            said.contains("where people sign in from elsewhere"),
            "{said}"
        );
    });
    assert!(here.received().is_empty(), "nothing reached it");
}

#[test]
fn an_address_is_an_origin_then_its_path() {
    assert_eq!(
        address(" HTTPS://MCP.Example.com/Mcp/ ").as_deref(),
        Some("https://mcp.example.com/Mcp")
    );
    assert_eq!(
        address("http://127.0.0.1:8080/mcp?team=a").as_deref(),
        Some("http://127.0.0.1:8080/mcp?team=a")
    );
    assert_eq!(
        address("https://mcp.example.com").as_deref(),
        Some("https://mcp.example.com")
    );
    for refused in [
        "http://mcp.example.com/mcp",
        "https://me@mcp.example.com/mcp",
        "mcp.example.com/mcp",
        "https://mcp.example.com/mcp#part",
        "https://mcp example.com",
        "ftp://mcp.example.com",
    ] {
        assert_eq!(address(refused), None, "{refused}");
    }
}
