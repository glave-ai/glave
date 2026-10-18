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
    use glave_ports::computer::Bytes;
    use glave_ports::testing::{FAKE_COMMIT, FakeGitHost, FakeHttp, fake_secret};
    use glave_provider_openai::fake::{FakeServer, Reply};
    use serde_json::json;

    use super::served::*;
    use super::support::*;

    /// The token the person pastes.
    const TOKEN: &str = "gta_e2e0token0never0shown0wxyz";

    /// What GitHub hands back: a GitHub App's user token once the person approves the sign-in,
    /// with the refresh token that renews it, and the pair that renewal brings.
    struct Tokens {
        signed_in: String,
        refresh: String,
        renewed: String,
        second_refresh: String,
    }

    impl Tokens {
        fn new() -> Self {
            Self {
                signed_in: fake_secret("ghu_"),
                refresh: fake_secret("ghr_"),
                renewed: fake_secret("ghu_"),
                second_refresh: fake_secret("ghr_"),
            }
        }

        /// GitHub's answer to the renewal: the renewed token and its own refresh token.
        fn renewal(&self) -> (u16, String) {
            let renewed = json!({"access_token": self.renewed, "token_type": "bearer",
                                 "expires_in": 28800, "refresh_token": self.second_refresh});
            (200, renewed.to_string())
        }

        /// Each by what it is, and the device code: none may show.
        fn kept_out(&self) -> [(&'static str, &str); 5] {
            [
                ("the token", self.signed_in.as_str()),
                ("the renewed token", self.renewed.as_str()),
                ("the device code", "dc-e2e"),
                ("the refresh token", self.refresh.as_str()),
                ("the second refresh token", self.second_refresh.as_str()),
            ]
        }
    }

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

    /// What GitHub says its app may do where the person installed it.
    fn installed() -> (u16, String) {
        let installation = json!({"id": 1, "repository_selection": "selected",
            "permissions": {"contents": "write", "pull_requests": "write", "metadata": "read"}});
        (
            200,
            json!({"total_count": 1, "installations": [installation]}).to_string(),
        )
    }

    /// GitHub's side of a sign-in in the browser at a fake, then `then`: the code, one poll
    /// still waiting, a GitHub App's user token about to expire with the refresh token that
    /// renews it, whom it is for, and what the app may do.
    fn github(tokens: &Tokens, then: Vec<(u16, String)>) -> FakeHttp {
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
                json!({"access_token": tokens.signed_in, "token_type": "bearer",
                       "expires_in": 60, "refresh_token": tokens.refresh})
                .to_string(),
            ),
            (200, json!({"login": "octocat"}).to_string()),
            installed(),
        ];
        FakeHttp::start([signed_in, then].concat()).unwrap()
    }

    /// Signs in through `github`'s app in the browser, as a client does: the id connected.
    fn signs_in(served: &mut Served, github: &FakeHttp) -> serde_json::Value {
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
        assert_eq!(credential["can"], json!(["read", "push", "pull_requests"]));
        let again = served.call("credential/device/poll", &json!({"flow": flow}));
        assert_eq!(again["error"]["code"], -32024, "a finished sign-in is gone");
        credential["id"].clone()
    }

    /// A renewed sign-in, kept under an id of its own, is still removed by the id it had.
    fn removes_by_its_first_id(served: &mut Served, signed_in: &serde_json::Value) {
        let listed = served.call("credential/list", &json!({}));
        let kept = &listed["result"]["credentials"];
        assert_eq!(kept.as_array().map(Vec::len), Some(1), "{listed}");
        assert_ne!(&kept[0]["id"], signed_in, "renewed under an id of its own");
        let removed = served.call("credential/remove", &json!({"credential": signed_in}));
        assert!(
            removed.get("error").is_none(),
            "the id it had names it: {removed}"
        );
        let listed = served.call("credential/list", &json!({}));
        assert_eq!(listed["result"]["credentials"], json!([]), "{listed}");
    }

    #[test]
    fn signing_in_to_github_in_the_browser_connects_it_and_renews_its_token() {
        let place = place("hosts-device");
        let tokens = Tokens::new();
        let then = vec![tokens.renewal(), (404, "{}".to_owned()), installed()];
        let github = github(&tokens, then);
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
        let signed_in = signs_in(&mut served, &github);
        let thread = served.call("thread/start", &json!({}))["result"]["thread"]["id"].clone();
        let turn = served.call(
            "turn/start",
            &json!({"thread": thread, "prompt": "branches?"}),
        );
        assert_eq!(turn["result"]["turn"]["outcome"], "completed", "{turn}");
        let results = said(&model.received()[1].body, "tool").concat();
        removes_by_its_first_id(&mut served, &signed_in);

        let said = served.end();
        for (what, secret) in tokens.kept_out() {
            assert!(!said.contains(secret), "{what} shows: {said}");
            assert!(!results.contains(secret), "{what} reached the model");
            assert!(
                !holds(&place.home, secret),
                "{what} is sealed at rest, and in no log"
            );
        }
        let asked = github.received();
        assert_eq!(asked[0].path, "/login/device/code");
        assert_eq!(asked[2].path, "/login/oauth/access_token");
        assert_eq!(asked[3].path, "/api/v3/user");
        let bearer = format!("Bearer {}", tokens.signed_in);
        assert_eq!(asked[3].headers.get("authorization"), Some(&bearer));
        assert_eq!(asked[4].path, "/api/v3/user/installations");
        assert_eq!(asked[4].headers.get("authorization"), Some(&bearer));
        let refresh = &asked[5];
        assert!(
            refresh.path == "/login/oauth/access_token"
                && refresh.body.contains("grant_type=refresh_token")
                && refresh
                    .body
                    .contains(&format!("refresh_token={}", tokens.refresh))
                && !refresh.body.contains("client_secret"),
            "renewed with the refresh token alone: {}",
            refresh.body
        );
        let basic = asked[6].headers.get("authorization").unwrap();
        let encoded = basic.strip_prefix("Basic ").unwrap();
        let sent: Bytes = serde_json::from_value(json!(encoded)).unwrap();
        let sent = String::from_utf8(sent.as_slice().to_vec()).unwrap();
        assert_eq!(
            sent,
            format!("octocat:{}", tokens.renewed),
            "git got the renewed token"
        );
        let renewed = format!("Bearer {}", tokens.renewed);
        assert_eq!(asked[7].headers.get("authorization"), Some(&renewed));
    }
}
