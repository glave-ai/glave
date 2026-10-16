//! What glave can do with a connected host, from what the host said its token may do
//! (`specs/session.md`, "Git hosts"): a classic GitHub token's scopes, from the header GitHub
//! answers with; a GitLab token's, asked of GitLab when it is connected; a GitHub App's
//! permissions on its installations, asked of GitHub each time, since the person may install
//! the app on more repositories after signing in. Where the host does not say, as for a
//! fine-grained GitHub token, Gitea's or Bitbucket's, nothing is claimed.

use std::time::Duration;

use glave_event::{Forge, Secret};
use glave_session::protocol::Ability;
use reqwest::header::HeaderMap;
use serde_json::Value;

use super::check::{Http, api, read};

/// How long asking GitHub about its app's installations may take, so listing stays quick.
const INSTALLATIONS: Duration = Duration::from_secs(5);
/// Everything glave does with a host, in the order clients show it.
const ALL: [Ability; 4] = [
    Ability::Read,
    Ability::Push,
    Ability::PullRequests,
    Ability::Issues,
];

/// A classic GitHub token's scopes, from the headers of GitHub's answer to it. Only for a
/// classic token or an OAuth app's (`ghp_`, `gho_`): for others the header says nothing.
pub fn github_scopes(headers: &HeaderMap, token: &Secret) -> Option<Vec<String>> {
    let classic = ["ghp_", "gho_"]
        .iter()
        .any(|prefix| token.expose().starts_with(prefix));
    let header = headers.get("x-oauth-scopes")?.to_str().ok()?;
    classic.then(|| {
        header
            .split(',')
            .map(str::trim)
            .filter(|scope| !scope.is_empty())
            .map(str::to_owned)
            .collect()
    })
}

/// A GitLab token's scopes, as GitLab says of the token it is asked with.
pub async fn gitlab_scopes(http: &Http, url: &str, token: &Secret) -> Option<Vec<String>> {
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
    Some(
        scopes
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
    )
}

/// What a GitHub App's user `token` lets glave do at GitHub at `url`: what the app may do on
/// the installations the person reaches, all of them together; nothing before it is installed.
pub async fn installations(http: &Http, url: &str, token: &Secret) -> Option<Vec<Ability>> {
    let asked = format!("{}/user/installations", api(url));
    let response = http
        .to(&asked)
        .get(&asked)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .bearer_auth(token.expose())
        .timeout(INSTALLATIONS)
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let answer = read(response).await?;
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
pub fn abilities(forge: Forge, scopes: Option<&[String]>) -> Option<Vec<Ability>> {
    let scopes = scopes?;
    let has = |scope: &str| scopes.iter().any(|given| given == scope);
    let can: &[Ability] = match forge {
        Forge::Github if has("repo") => &ALL,
        Forge::Github if has("public_repo") => return None,
        Forge::Gitlab if has("api") => &ALL,
        Forge::Gitlab if has("write_repository") => &[Ability::Read, Ability::Push],
        Forge::Gitlab if has("read_repository") => &[Ability::Read],
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
        assert_eq!(abilities(Forge::Gitea, Some(&scopes(&["all"]))), None);
        assert_eq!(abilities(Forge::Github, None), None, "a fine-grained token");
    }

    #[test]
    fn only_a_classic_token_s_scopes_header_is_read() {
        let mut headers = HeaderMap::new();
        headers.insert("x-oauth-scopes", "repo, workflow".parse().unwrap());
        let read = |token: &str| github_scopes(&headers, &Secret::new(token));
        assert_eq!(read("ghp_classic"), Some(scopes(&["repo", "workflow"])));
        assert_eq!(read("github_pat_fine"), None);
        assert_eq!(read("ghu_app"), None);
        assert_eq!(
            ordered(&[Ability::Issues, Ability::Read, Ability::Read]),
            [Ability::Read, Ability::Issues]
        );
    }
}
