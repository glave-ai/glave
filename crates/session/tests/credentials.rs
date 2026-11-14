//! Git hosts through the session API, over every transport: connecting one checks its token
//! and keeps it, never answering it back; signing in to GitHub in the browser gives a code and
//! then the host once approved; removing one destroys it. Only the person whose computer it is
//! may do this: the person on this computer, or anyone where people have computers of their
//! own, each their own (`people.rs`); anyone else signed in is refused, and a server whose
//! accounts someone else sets up refuses everyone.

mod people;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use glave_event::{CredentialId, Forge, PrincipalId, Secret};
use glave_ports::computer::{CredentialAsk, CredentialHost, GitCredential, GitCredentials};
use glave_ports::testing::any_secret;
use glave_ports::{BoxFuture, Clock as _, OrgId, SystemClock};
use glave_session::protocol::{ApiError, CredentialInfo, DeviceCode, Request, code};
use glave_session::signin::{LocalToken, SignIn};
use glave_session::{Answering, GitHosts, NewCredential, PeoplesGitHosts};
use serde_json::Value;

use crate::conformance::{Client, InProcess, OverHttp, OverJsonRpc};
use crate::support::issuer::fixed_clock;
use crate::support::web::{Web, serve_with};
use crate::support::*;

/// The token the fake host refuses; it takes any other.
const BAD: &str = "a-token-it-refuses";

/// Each person's hosts, kept apart by organization and person, each making ids of its own.
#[derive(Default)]
pub(crate) struct People {
    hosts: Mutex<BTreeMap<(OrgId, PrincipalId), Arc<Hosts>>>,
}

impl People {
    /// The hosts of `person` in `org`, made the first time.
    fn hosts_of(&self, org: OrgId, person: PrincipalId) -> Arc<Hosts> {
        let mut hosts = self.hosts.lock().unwrap();
        let seed = u8::try_from(hosts.len()).unwrap() + 1;
        let kept = hosts.entry((org, person)).or_insert_with(|| {
            Arc::new(Hosts {
                seed,
                ..Hosts::default()
            })
        });
        Arc::clone(kept)
    }
}

impl PeoplesGitHosts for People {
    fn of(
        &self,
        org: OrgId,
        person: PrincipalId,
    ) -> BoxFuture<'_, Result<Arc<dyn GitHosts>, ApiError>> {
        let hosts = self.hosts_of(org, person);
        Box::pin(async move { Ok(hosts as Arc<dyn GitHosts>) })
    }

    fn credentials(&self, org: OrgId, person: PrincipalId) -> Arc<dyn GitCredentials> {
        self.hosts_of(org, person)
    }
}

/// A host that keeps accounts in memory; its sign-in in the browser is approved on the
/// second poll.
#[derive(Default)]
pub(crate) struct Hosts {
    /// What sets its ids apart from another person's.
    seed: u8,
    kept: Mutex<Vec<CredentialInfo>>,
    polls: Mutex<u32>,
}

impl GitCredentials for Hosts {
    fn hosts(&self) -> BoxFuture<'_, Vec<CredentialHost>> {
        let kept = self.kept.lock().unwrap();
        let hosts = kept
            .iter()
            .filter_map(|info| CredentialHost::named(&info.url, info.forge))
            .collect();
        Box::pin(std::future::ready(hosts))
    }

    fn request<'a>(&'a self, ask: &'a CredentialAsk) -> BoxFuture<'a, Option<GitCredential>> {
        let kept = self.kept.lock().unwrap();
        let given = kept
            .iter()
            .filter_map(|info| CredentialHost::named(&info.url, info.forge))
            .find(|host| host.is_for(ask))
            .and_then(|_| {
                GitCredential::new("x-access-token", format!("token-{}", self.seed)).ok()
            });
        Box::pin(std::future::ready(given))
    }
}

impl Hosts {
    fn keep(&self, url: String, login: Option<String>) -> CredentialInfo {
        let mut kept = self.kept.lock().unwrap();
        let n = u8::try_from(kept.len()).unwrap();
        let mut bytes = [n + 1; 10];
        bytes[0] = self.seed;
        let info = CredentialInfo {
            id: CredentialId::new(SystemClock.now(), bytes).unwrap(),
            forge: Forge::Github,
            url,
            login,
            can: None,
            expired: false,
        };
        kept.push(info.clone());
        info
    }
}

impl GitHosts for Hosts {
    fn connect(&self, new: NewCredential) -> BoxFuture<'_, Result<CredentialInfo, ApiError>> {
        Box::pin(async move {
            if new.token.expose() == BAD {
                return Err(ApiError::new(
                    code::HOST,
                    "GitHub did not accept that token",
                ));
            }
            Ok(self.keep(new.url, new.username))
        })
    }

    fn list(&self) -> BoxFuture<'_, Result<Vec<CredentialInfo>, ApiError>> {
        Box::pin(async move { Ok(self.kept.lock().unwrap().clone()) })
    }

    fn remove(&self, credential: CredentialId) -> BoxFuture<'_, Result<(), ApiError>> {
        Box::pin(async move {
            let mut kept = self.kept.lock().unwrap();
            let before = kept.len();
            kept.retain(|info| info.id != credential);
            if kept.len() == before {
                return Err(ApiError::new(code::NO_CREDENTIAL, "no such account"));
            }
            Ok(())
        })
    }

    fn install(&self) -> Option<String> {
        Some("https://github.com/apps/glave/installations/new".into())
    }

    fn github(&self) -> String {
        "https://github.com".into()
    }

    fn start_device(&self) -> BoxFuture<'_, Result<DeviceCode, ApiError>> {
        Box::pin(async move {
            Ok(DeviceCode {
                flow: "flow-1".into(),
                code: "WDJB-MJHT".into(),
                url: "https://github.com/login/device".into(),
                expires_in: 900,
            })
        })
    }

    fn poll_device(&self, flow: String) -> BoxFuture<'_, Result<Option<CredentialInfo>, ApiError>> {
        Box::pin(async move {
            if flow != "flow-1" {
                return Err(ApiError::new(code::NO_CREDENTIAL, "no such sign-in"));
            }
            let mut polls = self.polls.lock().unwrap();
            *polls += 1;
            Ok((*polls >= 2)
                .then(|| self.keep("https://github.com".into(), Some("octocat".into()))))
        })
    }
}

/// A server whose git hosts the fake host keeps.
async fn fresh(dir: &Path) -> Tested {
    let mut setup = Setup::new(Vec::new());
    setup.hosts = Some(Arc::new(People::default()));
    setup.build(dir).await.0
}

fn connect(url: &str, token: &str) -> Request {
    Request::CredentialConnect {
        url: url.into(),
        token: Secret::new(token),
        username: None,
    }
}

#[test]
fn a_token_never_shows_in_a_request_s_debug() {
    let good = any_secret();
    let shown = format!("{:?}", connect("https://github.com", &good));
    assert!(!shown.contains(&good), "{shown}");
}

/// Connecting, signing in in the browser and removing, as the person on this computer.
async fn connect_and_remove(client: &mut impl Client) {
    let good = any_secret();
    let listed = client.call(Request::CredentialList {}).await.unwrap();
    assert_eq!(listed["credentials"], Value::Array(Vec::new()));
    assert_eq!(
        listed["install"],
        "https://github.com/apps/glave/installations/new"
    );

    let bad = client
        .call(connect("https://github.com", BAD))
        .await
        .unwrap_err();
    assert_eq!(bad.code, code::HOST);
    let connected = client
        .call(connect("https://github.com", &good))
        .await
        .unwrap();
    assert!(!connected.to_string().contains(&good), "{connected}");
    assert!(
        connected["credential"].get("hint").is_none(),
        "no token's ending"
    );
    assert_eq!(connected["credential"]["forge"], "github");

    let device = client
        .call(Request::CredentialDeviceStart {})
        .await
        .unwrap();
    assert_eq!(device["device"]["code"], "WDJB-MJHT");
    let poll = Request::CredentialDevicePoll {
        flow: "flow-1".into(),
    };
    let waiting = client.call(poll.clone()).await.unwrap();
    assert_eq!(waiting["flow"], "flow-1", "{waiting}");
    let signed_in = client.call(poll).await.unwrap();
    assert_eq!(signed_in["credential"]["login"], "octocat", "{signed_in}");
    let unknown = Request::CredentialDevicePoll {
        flow: "flow-2".into(),
    };
    let refused = client.call(unknown).await.unwrap_err();
    assert_eq!(refused.code, code::NO_CREDENTIAL);

    let listed = client.call(Request::CredentialList {}).await.unwrap();
    assert!(!listed.to_string().contains(&good), "{listed}");
    let id: CredentialId = serde_json::from_value(listed["credentials"][0]["id"].clone()).unwrap();
    client
        .call(Request::CredentialRemove { credential: id })
        .await
        .unwrap();
    let again = client
        .call(Request::CredentialRemove { credential: id })
        .await
        .unwrap_err();
    assert_eq!(again.code, code::NO_CREDENTIAL);
}

#[test]
fn the_person_here_connects_and_removes_a_git_host_over_each_local_transport() {
    block_on!(async {
        let dir = folder("hosts-in-process");
        let tested = fresh(&dir).await;
        connect_and_remove(&mut InProcess(tested.clone())).await;
        tested.shutdown().await;

        let dir = folder("hosts-json-rpc");
        let mut client = OverJsonRpc::new(fresh(&dir).await);
        connect_and_remove(&mut client).await;
        client.close().await;

        let dir = folder("hosts-socket");
        let mut client = OverJsonRpc::on_socket(fresh(&dir).await, "hosts").await;
        connect_and_remove(&mut client).await;
        client.close().await;

        let token = LocalToken::new([6; 32]);
        let signin = SignIn::local(token.clone(), Box::new(fixed_clock()));
        let dir = folder("hosts-http-local");
        let served = serve_with(fresh(&dir).await, signin, None).await;
        let mut web = Web::new(&served, Some(token.reveal().to_owned()));
        let good = any_secret();
        let answered = web
            .call(&connect("https://gitlab.example.com", &good))
            .await;
        assert_eq!(
            answered["result"]["credential"]["url"], "https://gitlab.example.com",
            "{answered}"
        );
        assert!(!answered.to_string().contains(&good), "{answered}");
        let told = web.call(&Request::Discover {}).await;
        assert_eq!(told["result"]["connects"]["git_hosts"], true, "{told}");
        // Someone not signed in learns what the server is, not what anyone may connect.
        let told = Web::new(&served, None).call(&Request::Discover {}).await;
        assert!(told["result"].get("connects").is_none(), "{told}");
        served.close().await;
    });
}

#[test]
fn a_caller_signed_in_elsewhere_may_not_connect_a_git_host() {
    block_on!(async {
        let dir = folder("hosts-oidc");
        let mut client = OverHttp::new(fresh(&dir).await).await;
        for request in [
            connect("https://github.com", &any_secret()),
            Request::CredentialList {},
            Request::CredentialDeviceStart {},
        ] {
            let refused = client.call(request).await.unwrap_err();
            assert_eq!(refused.code, code::NOT_ALLOWED, "{refused}");
            assert!(
                refused
                    .message
                    .to_string()
                    .contains("whoever runs this server connects"),
                "{refused}"
            );
        }
        client.close().await;
    });
}

#[test]
fn a_server_whose_accounts_are_set_up_for_it_refuses_git_hosts() {
    block_on!(async {
        let dir = folder("hosts-none");
        let tested = server(&dir, Vec::new(), Answering::Clients).await;
        let refused = tested
            .request(connect("https://github.com", &any_secret()))
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::NOT_AVAILABLE);
        tested.shutdown().await;
    });
}
