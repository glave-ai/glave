//! Renewals against a fake GitHub and a ledger in memory: one at a time, a refresh token spent
//! once, and nothing renewed that is no longer kept; and each sign-in's apart from another's.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use glave_config::policy::GithubFile;
use glave_event::{At, Forge, Principal, Renewal, Secret};
use glave_ports::computer::{CredentialAsk, GitCredentials as _};
use glave_ports::testing::{FakeHttp, fake_secret};
use glave_ports::{Clock as _, SystemClock};
use serde_json::json;

use super::*;
use crate::credentials::Hosts;

/// The app a test's sign-ins come through.
const APP: &str = "Iv23li0test";

/// A GitHub App's sign-in to `url`, about to expire.
fn signed_in(url: &str) -> CredentialConnected {
    let soon = SystemClock
        .now()
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
            client_id: APP.to_owned(),
        }),
        by: Principal::glave(),
    }
}

fn github(url: &str) -> Github {
    let file = GithubFile {
        client_id: APP.into(),
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
fn job(ledger: &Ledger, fake: &FakeHttp, current: &CredentialConnected) -> Renewing<HostRenewal> {
    let job = HostRenewal {
        ledger: ledger.clone(),
        http: Http::new().unwrap(),
        github: github(&fake.url),
        current: current.clone(),
    };
    Renewing { job, fresh: None }
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
    let renewals = Renewals::<CredentialConnected>::default();
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
    let renewals = Renewals::<CredentialConnected>::default();
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
    let renewals = Renewals::<CredentialConnected>::default();
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

/// A renewal of one sign-in that renews it as `fresh`, counting how often it spent its
/// refresh token, and that can keep it when `keeps`.
struct Counted {
    fresh: String,
    spent: Arc<AtomicUsize>,
    keeps: bool,
}

impl Renew for Counted {
    type Kept = String;

    fn url(&self) -> &str {
        &self.fresh
    }

    fn renewed(&self) -> impl Future<Output = Result<String, Ended<String>>> + Send {
        self.spent.fetch_add(1, Ordering::SeqCst);
        std::future::ready(Ok(self.fresh.clone()))
    }

    fn keep(&self, _fresh: &String) -> impl Future<Output = Result<bool, AccountsError>> + Send {
        let kept = if self.keeps {
            Ok(true)
        } else {
            Err(AccountsError::Event("the store is away".into()))
        };
        std::future::ready(kept)
    }
}

#[test]
fn each_sign_in_gets_only_its_own_renewal_and_an_unkept_one_is_kept_not_renewed_again() {
    let (one, two) = (
        super::super::new_id().unwrap(),
        super::super::new_id().unwrap(),
    );
    let spent = [Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0))];
    let job = |which: usize, keeps: bool| Counted {
        fresh: format!("renewed {which}"),
        spent: Arc::clone(&spent[which]),
        keeps,
    };
    let renewals = Renewals::<String>::default();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap();
    runtime.block_on(async {
        let unkept = renewals.wait(one, |fresh| {
            Some(Renewing {
                job: job(0, false),
                fresh,
            })
        });
        assert_eq!(
            unkept.await.as_deref(),
            Some("renewed 0"),
            "served from memory"
        );
        let other = renewals.wait(two, |fresh| {
            assert!(fresh.is_none(), "another's renewal is never handed over");
            Some(Renewing {
                job: job(1, true),
                fresh,
            })
        });
        assert_eq!(other.await, None, "its own, kept");
        let taken_up = std::sync::atomic::AtomicBool::new(false);
        let again = renewals.wait(one, |fresh| {
            assert_eq!(fresh.as_deref(), Some("renewed 0"));
            taken_up.store(true, Ordering::SeqCst);
            Some(Renewing {
                job: job(0, true),
                fresh,
            })
        });
        assert_eq!(again.await, None, "kept at last");
        assert!(
            taken_up.load(Ordering::SeqCst),
            "the renewal not kept is kept, not lost"
        );
    });
    let spent = spent.map(|spent| spent.load(Ordering::SeqCst));
    assert_eq!(spent, [1, 1], "each refresh token spent once");
}

#[test]
fn a_sign_in_renewed_waits_a_while_under_its_new_id_too() {
    let renewed = [
        super::super::new_id().unwrap(),
        super::super::new_id().unwrap(),
    ];
    let spent = Arc::new(AtomicUsize::new(0));
    let renewals = Renewals::<String>::default();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap();
    runtime.block_on(async {
        // Its first renewal kept under the second id, whose token lasts as little.
        for signed_in in renewed {
            let asked = renewals.wait(signed_in, |fresh| {
                let job = Counted {
                    fresh: "https://mcp.example.com/mcp".into(),
                    spent: Arc::clone(&spent),
                    keeps: true,
                };
                Some(Renewing { job, fresh })
            });
            assert_eq!(asked.await, None);
        }
    });
    let spent = spent.load(Ordering::SeqCst);
    assert_eq!(spent, 1, "renewed at most once a while, at one address");
}

#[test]
fn a_renewal_not_kept_goes_once_its_sign_in_is_no_longer_kept() {
    let signed_in = super::super::new_id().unwrap();
    let spent = Arc::new(AtomicUsize::new(0));
    let renewals = Renewals::<String>::default();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap();
    runtime.block_on(async {
        let read = Instant::now();
        let unkept = renewals.wait(signed_in, |fresh| {
            let job = Counted {
                fresh: "renewed".into(),
                spent: Arc::clone(&spent),
                keeps: false,
            };
            Some(Renewing { job, fresh })
        });
        assert_eq!(unkept.await.as_deref(), Some("renewed"));
        // A read begun before the renewal started can't tell it was renewed.
        renewals.only(read, |_| false);
        let kept = renewals.wait(signed_in, |fresh| {
            assert!(fresh.is_some(), "a read that began before it keeps it");
            None::<Renewing<Counted>>
        });
        assert_eq!(kept.await, None);
        // Removed by another glave on the same store.
        renewals.only(Instant::now(), |_| false);
        let again = renewals.wait(signed_in, |fresh| {
            assert!(
                fresh.is_none(),
                "the renewal not kept went with its sign-in"
            );
            None::<Renewing<Counted>>
        });
        assert_eq!(again.await, None);
    });
}

/// `url`'s `[github]` app, another than the one that issued [`signed_in`]'s sign-ins.
fn another_app(url: &str) -> GithubFile {
    GithubFile {
        client_id: "Iv23li0other".into(),
        app: None,
        url: Some(url.to_owned()),
    }
}

#[test]
fn a_sign_in_goes_only_to_the_app_that_issued_it_and_with_none_left_is_signed_out() {
    let fake = FakeHttp::start(Vec::new()).unwrap();
    let ledger = block_on(crate::accounts::in_memory()).unwrap();
    block_on(async {
        let first = signed_in(&fake.url);
        ledger.keep_credential(&first).await.unwrap();
        let hosts = Hosts::open(ledger.clone(), Some(&another_app(&fake.url)))
            .await
            .unwrap();
        let (protocol, host) = fake.url.split_once("://").unwrap();
        let ask = CredentialAsk {
            protocol: protocol.to_owned(),
            host: host.to_owned(),
            path: None,
        };

        // Good for a minute yet, and served as it is: nothing asks the app that didn't issue it.
        let served = hosts.request(&ask).await.unwrap();
        assert_eq!(served.password.expose(), first.token.expose());
        assert!(!hosts.signed_out(&first));

        let past = SystemClock
            .now()
            .checked_sub(jiff::SignedDuration::from_secs(60))
            .unwrap();
        let mut expired = first.clone();
        if let Some(renewal) = &mut expired.renewal {
            renewal.expires = At::from(past);
        }
        assert!(hosts.signed_out(&expired), "only signing in brings it back");
        assert!(hosts.renewer(&expired).is_none());
    });
    assert_eq!(fake.received().len(), 0, "no refresh token left this glave");
}

#[test]
fn a_sign_in_through_glave_s_own_app_renews_through_it_whatever_github_names_since() {
    let ledger = block_on(crate::accounts::in_memory()).unwrap();
    block_on(async {
        let hosts = Hosts::open(ledger, Some(&another_app("https://github.com")))
            .await
            .unwrap();
        let mut own = signed_in("https://github.com");
        if let Some(renewal) = &mut own.renewal {
            renewal.client_id = super::super::device::OWN_CLIENT_ID.to_owned();
        }
        assert!(hosts.renewer(&own).is_some());
        // Before glave kept who issued it: nothing renews it.
        if let Some(renewal) = &mut own.renewal {
            renewal.client_id = String::new();
        }
        assert!(hosts.renewer(&own).is_none());
    });
}
