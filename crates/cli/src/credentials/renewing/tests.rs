//! Renewals against a fake GitHub and a ledger in memory: one at a time, a refresh token spent
//! once, and nothing renewed that is no longer kept.

use glave_config::policy::GithubFile;
use glave_event::{At, Forge, Principal, Renewal, Secret};
use glave_ports::testing::{FakeHttp, fake_secret};
use serde_json::json;

use super::*;

/// A GitHub App's sign-in to `url`, about to expire.
fn signed_in(url: &str) -> CredentialConnected {
    let soon = jiff::Timestamp::now()
        .checked_add(jiff::SignedDuration::from_secs(60))
        .unwrap();
    CredentialConnected {
        credential: super::super::new_id().unwrap(),
        forge: Forge::Github,
        url: url.to_owned(),
        username: "octocat".into(),
        token: Secret::new(fake_secret("ghu_")),
        login: Some("octocat".into()),
        scopes: None,
        renewal: Some(Renewal {
            expires: At::from(soon),
            refresh: Secret::new(fake_secret("ghr_")),
        }),
        by: Principal::glave(),
    }
}

fn github(url: &str) -> Github {
    let file = GithubFile {
        client_id: "Iv23li0test".into(),
        app: None,
        url: Some(url.to_owned()),
    };
    Github::new(&file).unwrap()
}

/// What GitHub answers a renewal with: a new token, which it returns, and a new refresh token.
fn renewal_answer() -> (String, String) {
    let token = fake_secret("ghu_");
    let answer = json!({"access_token": token, "expires_in": 28800,
                        "refresh_token": fake_secret("ghr_")});
    (token, answer.to_string())
}

/// What renewing `current` through `fake` needs.
fn job(ledger: &Ledger, fake: &FakeHttp, current: &CredentialConnected) -> Renewing {
    Renewing {
        ledger: ledger.clone(),
        http: Http::new().unwrap(),
        github: github(&fake.url),
        current: current.clone(),
        fresh: None,
    }
}

fn block_on<T>(future: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

#[test]
fn asks_beside_each_other_share_one_renewal_and_a_renewed_token_is_not_renewed_again() {
    let (second, renewed) = renewal_answer();
    let fake = FakeHttp::start(vec![(200, renewed)]).unwrap();
    let ledger = block_on(crate::accounts::in_memory()).unwrap();
    let first = signed_in(&fake.url);
    let spent = format!(
        "refresh_token={}",
        first.renewal.as_ref().unwrap().refresh.expose()
    );
    let renewals = Renewals::default();
    block_on(async {
        ledger.keep_credential(&first).await.unwrap();
        let id = first.credential;
        let one = renewals.wait(id, |fresh| {
            Some(Renewing {
                fresh,
                ..job(&ledger, &fake, &first)
            })
        });
        let two = renewals.wait(id, |fresh| {
            Some(Renewing {
                fresh,
                ..job(&ledger, &fake, &first)
            })
        });
        assert!(matches!(tokio::join!(one, two), (None, None)));
        // A command that read the old sign-in before it was renewed starts no renewal.
        let late = renewals.wait(id, |_| Some(job(&ledger, &fake, &first)));
        assert!(late.await.is_none());
        let accounts = ledger.accounts().await.unwrap();
        let kept = accounts.credential(id).unwrap();
        assert_eq!(kept.token.expose(), second);
        assert_eq!(accounts.credentials.len(), 1);
    });
    let asked = fake.received();
    assert_eq!(asked.len(), 1, "the refresh token is spent once");
    assert!(asked[0].body.contains(&spent));
}

#[test]
fn a_renewal_github_refused_is_tried_again_only_after_a_while() {
    let refused = json!({"error": "bad_refresh_token", "error_description": "no\nlonger"});
    let fake = FakeHttp::start(vec![(200, refused.to_string())]).unwrap();
    let ledger = block_on(crate::accounts::in_memory()).unwrap();
    let first = signed_in(&fake.url);
    let renewals = Renewals::default();
    block_on(async {
        ledger.keep_credential(&first).await.unwrap();
        let id = first.credential;
        for _ in 0..2 {
            let waited = renewals.wait(id, |_| Some(job(&ledger, &fake, &first)));
            assert!(waited.await.is_none());
        }
        let accounts = ledger.accounts().await.unwrap();
        let kept = accounts.credential(id).unwrap();
        assert_eq!(kept.token.expose(), first.token.expose());
    });
    assert_eq!(fake.received().len(), 1);
}

#[test]
fn a_host_removed_before_its_renewal_is_not_renewed() {
    let (_, renewed) = renewal_answer();
    let fake = FakeHttp::start(vec![(200, renewed)]).unwrap();
    let ledger = block_on(crate::accounts::in_memory()).unwrap();
    let first = signed_in(&fake.url);
    let renewals = Renewals::default();
    block_on(async {
        ledger.keep_credential(&first).await.unwrap();
        assert!(ledger.remove_credential(first.credential).await.unwrap());
        let waited = renewals.wait(first.credential, |_| Some(job(&ledger, &fake, &first)));
        assert!(waited.await.is_none());
        assert_eq!(ledger.accounts().await.unwrap().credentials, []);
        // Unblocks the fake, which no renewal asked: what it is asked here is all it saw.
        let probe = format!("{}/probe", fake.url);
        let _ = reqwest::get(&probe).await;
    });
    let asked = fake.received();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].path, "/probe", "the refresh token was never spent");
}
