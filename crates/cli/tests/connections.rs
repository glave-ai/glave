//! `glave connect` and `glave connections`, the real binary through the person's server:
//! a token read from standard input, never an argument, checked with its host, listed with
//! what glave can do with it and never shown, and removed; GitHub signed in to in the browser.

use glave_ports::testing::{FakeGitHost, FakeHttp, any_secret, fake_secret};
use glave_provider_openai::fake::FakeServer;
use serde_json::json;

use crate::http::{only_the_server_answers, serve_http};
use crate::served::holds;
use crate::support::*;

/// The token the person pastes.
const TOKEN: &str = "gta_e2e0cli0token0never0shown0wxyz";

#[test]
fn a_host_is_connected_with_a_token_from_standard_input_listed_and_removed() {
    let place = place("connect-token");
    let host = FakeGitHost::start("ana", TOKEN).unwrap();
    let model = FakeServer::start(Vec::new()).unwrap();
    let (_served, _) = serve_http(&place, &model, &[]);
    only_the_server_answers(&place);
    let run = |args: &[&str], stdin: &str| {
        let output = glave(&place, &model, args, stdin);
        (output.status.success(), text(&output))
    };

    let (ok, said) = run(&["connect", &host.url, "--token", TOKEN], "");
    assert!(!ok && said.contains("looks like a token"), "{said}");
    assert!(!said.contains(TOKEN), "never repeated: {said}");
    let given = fake_secret("ghp_");
    let (ok, said) = run(&["connect", &given], "");
    assert!(!ok && said.contains("revoke it"), "{said}");
    assert!(!said.contains(&given), "never repeated: {said}");
    let (ok, said) = run(&["connect", "notanaddress"], "");
    assert!(!ok && said.contains("not a git host's address"), "{said}");
    let (ok, said) = run(&["connect", &host.url], "");
    assert!(!ok && said.contains("no token given"), "{said}");
    let (ok, said) = run(&["connect", &host.url], "gta_wrong0token\n");
    assert!(!ok && said.contains("did not accept that token"), "{said}");

    let (ok, said) = run(&["connect", &host.url], &format!("{TOKEN}\n"));
    assert!(ok, "{said}");
    let name = host.url.trim_start_matches("http://");
    assert!(
        said.contains(&format!(
            "Connected http://{name} (Gitea), signed in as ana"
        )) && said.contains("token ending wxyz"),
        "{said}"
    );
    assert!(
        said.contains("depends on the access you gave its token"),
        "{said}"
    );

    let (ok, listed) = run(&["connections"], "");
    assert!(
        ok && listed.contains("(Gitea), signed in as ana"),
        "{listed}"
    );
    let id = listed
        .lines()
        .find_map(|line| line.trim().strip_prefix("id "))
        .unwrap()
        .to_owned();
    let (ok, said) = run(&["connections", "remove", &id], "");
    assert!(ok && said.contains("Removed"), "{said}");
    let (ok, listed) = run(&["connections"], "");
    assert!(
        ok && listed.contains("No git host is connected"),
        "{listed}"
    );

    let (_, again) = run(&["connections", "remove", &id], "");
    assert!(
        again.contains("-32024") || again.contains("no git host"),
        "{again}"
    );
    assert!(!holds(&place.home, TOKEN), "sealed at rest, and in no log");
    for said in [&listed, &again] {
        assert!(!said.contains(TOKEN));
    }
}

#[test]
fn what_a_gitlab_token_may_do_is_what_gitlab_says() {
    let place = place("connect-gitlab");
    let gitlab = FakeHttp::start(vec![
        (200, json!({"username": "ana"}).to_string()),
        (
            200,
            json!({"scopes": ["read_repository", "write_repository"]}).to_string(),
        ),
    ])
    .unwrap();
    let model = FakeServer::start(Vec::new()).unwrap();
    let (_served, _) = serve_http(&place, &model, &[]);
    only_the_server_answers(&place);
    let token = any_secret();
    let output = glave(
        &place,
        &model,
        &["connect", &gitlab.url],
        &format!("{token}\n"),
    );
    let said = text(&output);
    assert!(output.status.success(), "{said}");
    assert!(said.contains("(GitLab), signed in as ana"), "{said}");
    assert!(
        said.contains("Can read and clone your repositories, push branches."),
        "{said}"
    );
    let asked = gitlab.received();
    assert_eq!(asked[0].path, "/api/v4/user");
    assert_eq!(asked[1].path, "/api/v4/personal_access_tokens/self");
    assert_eq!(asked[1].headers.get("private-token"), Some(&token));
}

#[test]
fn github_is_signed_in_to_in_the_browser_from_the_terminal() {
    let place = place("connect-github");
    // A GitHub App's token, whose abilities come from the app's installations.
    let signed_in = fake_secret("ghu_");
    let installed = json!({"total_count": 1, "installations": [{"id": 1,
            "permissions": {"contents": "read", "issues": "write"}}]});
    let github = FakeHttp::start(vec![
        (
            200,
            json!({"device_code": "dc-cli", "user_code": "WDJB-MJHT",
                       "verification_uri": "https://github.com/login/device",
                       "expires_in": 900, "interval": 0})
            .to_string(),
        ),
        (200, json!({"error": "authorization_pending"}).to_string()),
        (
            200,
            json!({"access_token": signed_in, "token_type": "bearer"}).to_string(),
        ),
        (200, json!({"login": "octocat"}).to_string()),
        (200, installed.to_string()),
        (200, installed.to_string()),
    ])
    .unwrap();
    std::fs::create_dir_all(&place.home).unwrap();
    let app = format!(
        "[github]\nclient_id = \"Iv23li0e2e\"\napp = \"glave-e2e\"\nurl = \"{}\"\n",
        github.url
    );
    std::fs::write(place.home.join("config.toml"), app).unwrap();
    let model = FakeServer::start(Vec::new()).unwrap();
    let (_served, _) = serve_http(&place, &model, &[]);
    only_the_server_answers(&place);

    let output = glave(&place, &model, &["connect", "github"], "");
    let said = text(&output);
    assert!(output.status.success(), "{said}");
    assert!(
        said.contains("Open https://github.com/login/device and enter the code WDJB-MJHT"),
        "{said}"
    );
    assert!(said.contains("(GitHub), signed in as octocat"), "{said}");
    assert!(
        said.contains("Can read and clone your repositories, open and comment on issues."),
        "{said}"
    );
    let install = format!("{}/github-apps/glave-e2e/installations/new", github.url);
    assert!(said.contains(&install), "{said}");
    assert!(!said.contains(&signed_in) && !said.contains("dc-cli"));
    assert_eq!(github.received().len(), 6);
}
