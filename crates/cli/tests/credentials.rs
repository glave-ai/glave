//! Git hosts, through the real binary over stdio with a fresh home: a pasted token is checked
//! with its host, kept sealed, never shown, logged or answered back, and the next turn's
//! commands sign in with it, asking at each use; signing in to GitHub in the browser goes
//! through GitHub's device flow to the same end, and its token is renewed before it expires.

#[cfg(test)]
#[path = "support/binary.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
#[path = "support/served.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod served;

#[cfg(test)]
mod hosts {
    use glave_ports::Bytes;
    use glave_ports::testing::{FAKE_COMMIT, FakeGitHost, FakeHttp};
    use glave_provider_openai::fake::{FakeServer, Reply};
    use serde_json::json;

    use super::served::*;
    use super::support::*;

    /// The token the person pastes.
    const TOKEN: &str = "gta_e2e0token0never0shown0wxyz";
    /// The token GitHub hands back after the person approves the sign-in.
    const SIGNED_IN: &str = "ghu_e2e0signed0in0never0shown";
    /// The token that renews it.
    const RENEWED: &str = "ghu_e2e0renewed0never0shown";

    /// The command the model runs: git asking the fake host for its branches.
    fn fetch(host: &FakeGitHost) -> String {
        format!(
            "GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null GIT_TERMINAL_PROMPT=0 \
             git ls-remote {}/repo.git",
            host.url
        )
    }

    #[test]
    fn a_pasted_token_is_checked_kept_sealed_and_signs_the_next_command_in() {
        let place = place("hosts-pasted");
        let host = FakeGitHost::start("ana", TOKEN).unwrap();
        let model = FakeServer::start(vec![
            Reply::tool_call("call_1", "bash", &json!({"command": fetch(&host)})),
            Reply::text("Listed."),
        ])
        .unwrap();
        let args = ["--model", "m", "--sandbox", "off", "--approval", "auto"];
        let mut served = Served::start(&place, &model, &args);

        let refused = served.call(
            "credential/connect",
            &json!({"url": host.url, "token": "gta_wrong0token"}),
        );
        assert_eq!(refused["error"]["code"], -32025, "{refused}");
        let message = refused["error"]["message"].as_str().unwrap();
        assert!(message.contains("did not accept that token"), "{message}");

        let connected = served.call(
            "credential/connect",
            &json!({"url": format!("{}/ana/repo.git", host.url), "token": TOKEN}),
        );
        let credential = &connected["result"]["credential"];
        assert_eq!(credential["forge"], "gitea", "{connected}");
        assert_eq!(credential["login"], "ana");
        assert_eq!(credential["url"], host.url.as_str(), "kept as the host");
        assert_eq!(credential["hint"], "wxyz");

        let thread = served.call("thread/start", &json!({}))["result"]["thread"]["id"].clone();
        let turn = served.call(
            "turn/start",
            &json!({"thread": thread, "prompt": "branches?"}),
        );
        assert_eq!(turn["result"]["turn"]["outcome"], "completed", "{turn}");
        let received = model.received();
        let system = said(&received[0].body, "system").concat();
        let name = host.url.trim_start_matches("http://");
        assert!(
            system.contains(&format!("`tea` is signed in for {name}")),
            "the model is told what is signed in: {system}"
        );
        let results = said(&received[1].body, "tool").concat();
        assert!(results.contains(FAKE_COMMIT), "git signed in: {results}");

        let listed = served.call("credential/list", &json!({}));
        assert_eq!(listed["result"]["device"], false, "no GitHub app set up");
        let id = listed["result"]["credentials"][0]["id"].clone();
        let removed = served.call("credential/remove", &json!({"credential": id}));
        assert_eq!(removed["result"], json!({}), "{removed}");
        let listed = served.call("credential/list", &json!({}));
        assert_eq!(listed["result"]["credentials"], json!([]));

        let said = served.end();
        for secret in [TOKEN, "gta_wrong0token"] {
            assert!(!said.contains(secret), "never answered or logged: {said}");
            assert!(!holds(&place.home, secret), "sealed at rest, and in no log");
        }
        assert!(!results.contains(TOKEN), "{results}");
    }

    /// GitHub's side of a sign-in in the browser at a fake, then `then`: the code, one poll
    /// still waiting, a GitHub App's user token about to expire with the refresh token that
    /// renews it, and whom it is for.
    fn github(then: Vec<(u16, String)>) -> FakeHttp {
        let signed_in = vec![
            (
                200,
                json!({"device_code": "dc-e2e", "user_code": "WDJB-MJHT",
                       "verification_uri": "https://github.com/login/device",
                       "expires_in": 900, "interval": 0})
                .to_string(),
            ),
            (200, json!({"error": "authorization_pending"}).to_string()),
            (
                200,
                json!({"access_token": SIGNED_IN, "token_type": "bearer", "expires_in": 60,
                       "refresh_token": "ghr_e2e0refresh0never0shown"})
                .to_string(),
            ),
            (200, json!({"login": "octocat"}).to_string()),
        ];
        FakeHttp::start([signed_in, then].concat()).unwrap()
    }

    /// Signs in through `github`'s app in the browser, as a client does.
    fn signs_in(served: &mut Served, github: &FakeHttp) {
        let listed = served.call("credential/list", &json!({}));
        assert_eq!(listed["result"]["device"], true, "{listed}");
        assert_eq!(
            listed["result"]["install"],
            format!("{}/github-apps/glave-e2e/installations/new", github.url)
        );
        let started = served.call("credential/device/start", &json!({}));
        let device = &started["result"]["device"];
        assert_eq!(device["code"], "WDJB-MJHT", "{started}");
        assert_eq!(device["url"], "https://github.com/login/device");
        let flow = device["flow"].clone();
        let waiting = served.call("credential/device/poll", &json!({"flow": flow}));
        assert_eq!(waiting["result"]["flow"], flow, "{waiting}");
        let connected = served.call("credential/device/poll", &json!({"flow": flow}));
        let credential = &connected["result"]["credential"];
        assert_eq!(credential["forge"], "github", "{connected}");
        assert_eq!(credential["login"], "octocat");
        let again = served.call("credential/device/poll", &json!({"flow": flow}));
        assert_eq!(again["error"]["code"], -32024, "a finished sign-in is gone");
    }

    #[test]
    fn signing_in_to_github_in_the_browser_connects_it_and_renews_its_token() {
        let place = place("hosts-device");
        let github = github(vec![
            (
                200,
                json!({"access_token": RENEWED, "token_type": "bearer", "expires_in": 28800,
                       "refresh_token": "ghr_e2e0second0refresh"})
                .to_string(),
            ),
            (404, "{}".to_owned()),
        ]);
        std::fs::create_dir_all(&place.home).unwrap();
        let app = format!(
            "[github]\nclient_id = \"Iv23li0e2e\"\napp = \"glave-e2e\"\nurl = \"{}\"\n",
            github.url
        );
        std::fs::write(place.home.join("config.toml"), app).unwrap();
        // git sends its credential with its first request, so the fake sees which it got.
        let fetch = format!(
            "GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null GIT_TERMINAL_PROMPT=0 \
             git -c http.proactiveAuth=basic ls-remote {}/repo.git; true",
            github.url
        );
        let model = FakeServer::start(vec![
            Reply::tool_call("call_1", "bash", &json!({"command": fetch})),
            Reply::text("Listed."),
        ])
        .unwrap();
        let args = ["--model", "m", "--sandbox", "off", "--approval", "auto"];
        let mut served = Served::start(&place, &model, &args);
        signs_in(&mut served, &github);
        let thread = served.call("thread/start", &json!({}))["result"]["thread"]["id"].clone();
        let turn = served.call(
            "turn/start",
            &json!({"thread": thread, "prompt": "branches?"}),
        );
        assert_eq!(turn["result"]["turn"]["outcome"], "completed", "{turn}");
        let results = said(&model.received()[1].body, "tool").concat();

        let said = served.end();
        for secret in [SIGNED_IN, RENEWED, "dc-e2e", "ghr_e2e0"] {
            assert!(!said.contains(secret), "{secret} never shows: {said}");
            assert!(!results.contains(secret), "{results}");
        }
        assert!(!holds(&place.home, SIGNED_IN) && !holds(&place.home, RENEWED));
        let asked = github.received();
        assert_eq!(asked[0].path, "/login/device/code");
        assert_eq!(asked[2].path, "/login/oauth/access_token");
        assert_eq!(asked[3].path, "/api/v3/user");
        let bearer = format!("Bearer {SIGNED_IN}");
        assert_eq!(asked[3].headers.get("authorization"), Some(&bearer));
        let refresh = &asked[4];
        assert!(
            refresh.path == "/login/oauth/access_token"
                && refresh.body.contains("grant_type=refresh_token")
                && refresh
                    .body
                    .contains("refresh_token=ghr_e2e0refresh0never0shown")
                && !refresh.body.contains("client_secret"),
            "renewed with the refresh token alone: {}",
            refresh.body
        );
        let basic = asked[5].headers.get("authorization").unwrap();
        let encoded = basic.strip_prefix("Basic ").unwrap();
        let sent: Bytes = serde_json::from_value(json!(encoded)).unwrap();
        let sent = String::from_utf8(sent.as_slice().to_vec()).unwrap();
        assert_eq!(
            sent,
            format!("octocat:{RENEWED}"),
            "git got the renewed token"
        );
    }
}
