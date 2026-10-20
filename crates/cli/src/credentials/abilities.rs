//! What glave can do with a connected host, from what the host says its token may do
//! (`specs/session.md`, "Git hosts"): a classic GitHub token's scopes, from the header GitHub
//! answers with, asked again each time since they can be changed; a GitLab token's, asked of
//! GitLab when it is connected, since they cannot; a GitHub App's permissions on its
//! installations, asked of GitHub each time, since the person may install the app on more
//! repositories after signing in. Where the host does not say, as for a fine-grained GitHub
//! token, Gitea's or Bitbucket's, nothing is claimed.

use std::time::Duration;

use glave_event::{CredentialConnected, Forge, Secret};
use glave_session::protocol::Ability;
use reqwest::header::HeaderMap;
use serde_json::Value;

use super::check::{Http, api, read};

/// How long asking GitHub about a token may take while listing, so listing stays quick.
const LISTING: Duration = Duration::from_secs(5);
/// The scopes glave reads, the only ones it keeps: GitHub's, then GitLab's.
const KNOWN: [&str; 6] = [
    "repo",
    "public_repo",
    "api",
    "write_repository",
    "read_repository",
    "read_api",
];
/// Everything glave does with a host, in the order clients show it.
const ALL: [Ability; 4] = [
    Ability::Read,
    Ability::Push,
    Ability::PullRequests,
    Ability::Issues,
];

/// What glave can do with `credential` now: a GitHub App's token as GitHub says of the app's
/// installations, a classic GitHub token as GitHub says of its scopes (else as it said when it
/// was connected), any other as its host said when it was connected.
pub(super) async fn now(http: &Http, credential: &CredentialConnected) -> Option<Vec<Ability>> {
    let token = &credential.token;
    if credential.forge == Forge::Github && token.expose().starts_with("ghu_") {
        let expired = credential
            .renewal
            .as_ref()
            .is_some_and(|renewal| renewal.expires.timestamp() <= jiff::Timestamp::now());
        if expired {
            return None;
        }
        return installations(http, &credential.url, token).await;
    }
    if credential.forge == Forge::Github && classic(token) {
        let asked = format!("{}/user", api(&credential.url));
        let scopes = match asking(http, &asked, token).await {
            Some(response) => github_scopes(response.headers(), token),
            None => credential.scopes.clone(),
        };
        return abilities(Forge::Github, scopes.as_deref());
    }
    abilities(credential.forge, credential.scopes.as_deref())
}

/// Whether `token` is a classic GitHub token or an OAuth app's (`ghp_`, `gho_`), whose scopes
/// GitHub says in a header.
fn classic(token: &Secret) -> bool {
    ["ghp_", "gho_"]
        .iter()
        .any(|prefix| token.expose().starts_with(prefix))
}

/// GitHub's answer at `url` to `token`, within [`LISTING`], when it accepted the token.
async fn asking(http: &Http, url: &str, token: &Secret) -> Option<reqwest::Response> {
    let response = http
        .to(url)
        .get(url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .bearer_auth(token.expose())
        .timeout(LISTING)
        .send()
        .await
        .ok()?;
    response.status().is_success().then_some(response)
}

/// A classic GitHub token's scopes that glave reads, from the headers of GitHub's answer to
/// it. None for any other token: for others the header says nothing.
pub(super) fn github_scopes(headers: &HeaderMap, token: &Secret) -> Option<Vec<String>> {
    let header = headers.get("x-oauth-scopes")?.to_str().ok()?;
    classic(token).then(|| known(header.split(',').map(str::trim)))
}

/// The scopes among `scopes` that glave reads.
fn known<'a>(scopes: impl Iterator<Item = &'a str>) -> Vec<String> {
    scopes
        .filter(|scope| KNOWN.contains(scope))
        .map(str::to_owned)
        .collect()
}

/// A GitLab token's scopes, as GitLab says of the token it is asked with.
pub(super) async fn gitlab_scopes(http: &Http, url: &str, token: &Secret) -> Option<Vec<String>> {
    let asked = format!("{url}/api/v4/personal_access_tokens/self");
    let response = http
        .to(&asked)
        .get(&asked)
        .header("PRIVATE-TOKEN", token.expose())
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let scopes = read(response).await?.get("scopes")?.as_array()?.clone();
    Some(known(scopes.iter().filter_map(Value::as_str)))
}

/// What a GitHub App's user `token` lets glave do at GitHub at `url`: what the app may do on
/// the installations the person reaches, all of them together; nothing before it is installed.
pub(super) async fn installations(http: &Http, url: &str, token: &Secret) -> Option<Vec<Ability>> {
    let asked = format!("{}/user/installations", api(url));
    let answer = read(asking(http, &asked, token).await?).await?;
    let mut can = Vec::new();
    for installation in answer.get("installations")?.as_array()? {
        let permission = |name: &str| {
            installation
                .get("permissions")
                .and_then(|permissions| permissions.get(name))
                .and_then(Value::as_str)
        };
        let writes = |name: &str| matches!(permission(name), Some("write" | "admin"));
        if permission("contents").is_some() {
            can.push(Ability::Read);
        }
        if writes("contents") {
            can.push(Ability::Push);
        }
        if writes("pull_requests") {
            can.push(Ability::PullRequests);
        }
        if writes("issues") {
            can.push(Ability::Issues);
        }
    }
    Some(ordered(&can))
}

/// What glave can do on `forge` with a token the host said may do `scopes`; none when it did
/// not say, or said only what glave cannot tell apart (a classic token for public
/// repositories alone).
pub(super) fn abilities(forge: Forge, scopes: Option<&[String]>) -> Option<Vec<Ability>> {
    let scopes = scopes?;
    let has = |scope: &str| scopes.iter().any(|given| given == scope);
    let can: &[Ability] = match forge {
        Forge::Github if has("repo") => &ALL,
        Forge::Github if has("public_repo") => return None,
        Forge::Gitlab if has("api") => &ALL,
        Forge::Gitlab if has("write_repository") => &[Ability::Read, Ability::Push],
        Forge::Gitlab if has("read_repository") => &[Ability::Read],
        // What it allows of repositories GitLab's documentation does not say plainly.
        Forge::Gitlab if has("read_api") => return None,
        Forge::Github | Forge::Gitlab => &[],
        Forge::Gitea | Forge::Bitbucket | Forge::Other => return None,
    };
    Some(can.to_vec())
}

/// `can` without repeats, in the order clients show it.
fn ordered(can: &[Ability]) -> Vec<Ability> {
    ALL.into_iter()
        .filter(|ability| can.contains(ability))
        .collect()
}

#[cfg(test)]
mod tests {
    use glave_ports::testing::fake_secret;

    use super::*;

    fn scopes(given: &[&str]) -> Vec<String> {
        given.iter().map(|scope| (*scope).to_owned()).collect()
    }

    #[test]
    fn a_token_s_scopes_say_what_glave_can_do_and_nothing_is_claimed_unsaid() {
        let github = |given: &[&str]| abilities(Forge::Github, Some(&scopes(given)));
        assert_eq!(github(&["repo", "read:org"]), Some(ALL.to_vec()));
        assert_eq!(github(&["read:user"]), Some(Vec::new()));
        assert_eq!(github(&["public_repo"]), None);
        let gitlab = |given: &[&str]| abilities(Forge::Gitlab, Some(&scopes(given)));
        assert_eq!(gitlab(&["api"]), Some(ALL.to_vec()));
        assert_eq!(
            gitlab(&["read_repository", "write_repository"]),
            Some(vec![Ability::Read, Ability::Push])
        );
        assert_eq!(gitlab(&["read_repository"]), Some(vec![Ability::Read]));
        assert_eq!(gitlab(&["read_api"]), None);
        assert_eq!(abilities(Forge::Gitea, Some(&scopes(&["all"]))), None);
        assert_eq!(abilities(Forge::Github, None), None, "a fine-grained token");
    }

    #[test]
    fn only_a_classic_token_s_scopes_header_is_read() {
        let mut headers = HeaderMap::new();
        headers.insert("x-oauth-scopes", "repo, workflow".parse().unwrap());
        let read = |prefix: &str| github_scopes(&headers, &Secret::new(fake_secret(prefix)));
        assert_eq!(
            read("ghp_"),
            Some(scopes(&["repo"])),
            "only what glave reads"
        );
        assert_eq!(read("github_pat_"), None);
        assert_eq!(read("ghu_"), None);
        assert_eq!(
            ordered(&[Ability::Issues, Ability::Read, Ability::Read]),
            [Ability::Read, Ability::Issues]
        );
    }
}
