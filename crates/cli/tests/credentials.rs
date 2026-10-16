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
    use glave_ports::testing::{FAKE_COMMIT, FakeGitHost, FakeHttp};
    use glave_provider_openai::fake::{FakeServer, Reply};
    use serde_json::{Value, json};

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

    /// A command that runs `gh`, a stand-in found after glave's wrapper that shows the token
    /// it got for GitHub Enterprise in capitals, so the token itself never shows.
    const STAND_IN_GH: &str = "mkdir -p tools\ncat > tools/gh <<'FAKE'\n#!/bin/sh\n\
        printf 'gh sees: %s\\n' \"$(printf '%s' \"${GH_ENTERPRISE_TOKEN:-unset}\" | tr a-z A-Z)\"\n\
        FAKE\nchmod +x tools/gh\nPATH=\"${PATH%%:*}:$PWD/tools:${PATH#*:}\"\ngh pr list";

    #[test]
    fn signing_in_to_github_in_the_browser_connects_it_and_renews_its_token() {
        let place = place("hosts-device");
        let github = FakeHttp::start(vec![
            (
                200,
                json!({"device_code": "dc-e2e", "user_code": "WDJB-MJHT",
                       "verification_uri": "https://github.com/login/device",
                       "expires_in": 900, "interval": 0})
                .to_string(),
            ),
            (200, json!({"error": "authorization_pending"}).to_string()),
            // A GitHub App's user token: about to expire, with the refresh token that renews it.
            (
                200,
                json!({"access_token": SIGNED_IN, "token_type": "bearer", "expires_in": 60,
                       "refresh_token": "ghr_e2e0refresh0never0shown"})
                .to_string(),
            ),
            (200, json!({"login": "octocat"}).to_string()),
            (
                200,
                json!({"access_token": RENEWED, "token_type": "bearer", "expires_in": 28800,
                       "refresh_token": "ghr_e2e0second0refresh"})
                .to_string(),
            ),
        ])
        .unwrap();
        std::fs::create_dir_all(&place.home).unwrap();
        std::fs::write(
            place.home.join("config.toml"),
            format!(
                "[github]\nclient_id = \"Iv23li0e2e\"\napp = \"glave-e2e\"\nurl = \"{}\"\n",
                github.url
            ),
        )
        .unwrap();
        let model = FakeServer::start(vec![
            Reply::tool_call("call_1", "bash", &json!({"command": STAND_IN_GH})),
            Reply::text("Listed."),
        ])
        .unwrap();
        let args = ["--model", "m", "--sandbox", "off", "--approval", "auto"];
        let mut served = Served::start(&place, &model, &args);

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

        let thread = served.call("thread/start", &json!({}))["result"]["thread"]["id"].clone();
        let turn = served.call("turn/start", &json!({"thread": thread, "prompt": "prs?"}));
        assert_eq!(turn["result"]["turn"]["outcome"], "completed", "{turn}");
        let results = said(&model.received()[1].body, "tool").concat();
        let seen = format!("gh sees: {}", RENEWED.to_ascii_uppercase());
        assert!(results.contains(&seen), "renewed before use: {results}");

        let said = served.end();
        for secret in [SIGNED_IN, RENEWED, "dc-e2e", "ghr_e2e0"] {
            assert!(!said.contains(secret), "{secret} never shows: {said}");
            assert!(!results.contains(secret), "{results}");
        }
        assert!(!holds(&place.home, SIGNED_IN) && !holds(&place.home, RENEWED));
        let asked: Vec<(String, String, Value, String)> = github
            .received()
            .into_iter()
            .map(|r| {
                let auth = json!(r.headers.get("authorization"));
                (r.method, r.path, auth, r.body)
            })
            .collect();
        assert_eq!(asked[0].1, "/login/device/code");
        assert_eq!(asked[2].1, "/login/oauth/access_token");
        assert_eq!(
            (&asked[3].0, &asked[3].1, &asked[3].2),
            (
                &"GET".to_owned(),
                &"/api/v3/user".to_owned(),
                &json!(format!("Bearer {SIGNED_IN}"))
            )
        );
        assert_eq!(asked[4].1, "/login/oauth/access_token");
        assert!(
            asked[4].3.contains("grant_type=refresh_token")
                && asked[4]
                    .3
                    .contains("refresh_token=ghr_e2e0refresh0never0shown")
                && !asked[4].3.contains("client_secret"),
            "renewed with the refresh token alone: {}",
            asked[4].3
        );
    }
}
