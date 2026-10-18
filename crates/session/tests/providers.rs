//! Model accounts through the session API, over every transport: a server with nothing set up
//! runs no model; connecting an account checks its key, keeps it, and offers its models at
//! once; the key is never answered back; removing the account takes its models away. Only the
//! person on this computer may do this: a caller signed in with OIDC is refused, and a server
//! whose models someone else sets up refuses everyone.

use std::sync::{Arc, Mutex};

use glave_event::{ProviderId, ProviderKind, Secret};
use glave_harness::Settings;
use glave_ports::testing::FAKE_SECRET;
use glave_ports::{BoxFuture, Failure};
use glave_session::protocol::{ApiError, ErrorData, ModelInfo, ProviderInfo, Request, code, hint};
use glave_session::signin::{LocalToken, SignIn};
use glave_session::{Answering, Models, NewProvider, Providers};
use serde_json::Value;

use crate::conformance::{Client, InProcess, OverHttp, OverJsonRpc};
use crate::support::issuer::fixed_clock;
use crate::support::web::{Web, serve_with};
use crate::support::*;

/// The key the fake service refuses.
const BAD: &str = "a-key-it-refuses";
/// A key it takes.
const GOOD: &str = FAKE_SECRET;

/// A host that keeps accounts in memory, as a service that offers two models.
#[derive(Default)]
struct Accounts(Mutex<Vec<ProviderInfo>>);

impl Accounts {
    fn models(&self) -> Models {
        let kept = self.0.lock().unwrap();
        match kept.first() {
            None => Models::default(),
            Some(_) => Models {
                default: Some(("fake-big".into(), "fake".into())),
                offered: vec![ModelInfo::new("fake-small")],
            },
        }
    }
}

impl Providers for Accounts {
    fn connect(&self, new: NewProvider) -> BoxFuture<'_, Result<(ProviderInfo, Models), ApiError>> {
        Box::pin(async move {
            if new.key.expose() == BAD {
                return Err(ApiError {
                    code: code::PROVIDER,
                    message: "the service did not accept the key".into(),
                    data: Some(ErrorData {
                        failure: Failure::Refused,
                    }),
                });
            }
            let n = u8::try_from(self.0.lock().unwrap().len()).unwrap();
            let info = ProviderInfo {
                id: ProviderId::new(jiff::Timestamp::now(), [n + 1; 10]).unwrap(),
                kind: new.kind,
                url: new.url,
                hint: hint(new.key.expose()),
                models: vec!["fake-big".into(), "fake-small".into()],
            };
            self.0.lock().unwrap().push(info.clone());
            Ok((info, self.models()))
        })
    }

    fn list(&self) -> BoxFuture<'_, Result<Vec<ProviderInfo>, ApiError>> {
        Box::pin(async move { Ok(self.0.lock().unwrap().clone()) })
    }

    fn remove(&self, provider: ProviderId) -> BoxFuture<'_, Result<Models, ApiError>> {
        Box::pin(async move {
            {
                let mut kept = self.0.lock().unwrap();
                let before = kept.len();
                kept.retain(|info| info.id != provider);
                if kept.len() == before {
                    return Err(ApiError::new(code::NO_PROVIDER, "no such model account"));
                }
            }
            Ok(self.models())
        })
    }
}

/// A server with no model set up, whose accounts the fake host keeps.
async fn fresh(name: &str) -> Tested {
    let mut setup = Setup::asking(Vec::new(), Answering::Clients);
    setup.settings = Settings::new("", "");
    setup.providers = Some(Arc::new(Accounts::default()));
    setup.build(&folder(name)).await.0
}

#[test]
fn a_key_never_shows_in_a_request_s_debug() {
    let shown = format!("{:?}", connect(ProviderKind::Openai, GOOD));
    assert!(!shown.contains(GOOD), "{shown}");
}

fn connect(kind: ProviderKind, key: &str) -> Request {
    Request::ProviderConnect {
        kind,
        key: Secret::new(key),
        url: None,
    }
}

fn ids(models: &Value) -> Vec<String> {
    models["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|model| model["id"].as_str().unwrap().to_owned())
        .collect()
}

/// The whole first run, as the person on this computer.
async fn first_run(client: &mut impl Client) {
    let none = client.call(Request::ModelList {}).await.unwrap();
    assert!(ids(&none).is_empty(), "nothing is set up: {none}");
    let started = client
        .call(Request::ThreadStart {
            space: Some(scope().space),
            worktree: None,
            background: false,
            computer: None,
            model: None,
            approval: None,
        })
        .await
        .unwrap();
    let thread = serde_json::from_value(started["thread"]["id"].clone()).unwrap();
    let refused = client
        .call(Request::TurnStart {
            thread,
            prompt: "hi".into(),
            background: false,
            budget: None,
            model: None,
            approval: None,
            meta: glave_session::protocol::Meta::default(),
        })
        .await
        .unwrap_err();
    assert_eq!(refused.code, code::NO_MODEL);
    assert!(
        refused.message.contains("connect a model account"),
        "{refused}"
    );

    let bad = client
        .call(connect(ProviderKind::Openai, BAD))
        .await
        .unwrap_err();
    assert_eq!(bad.code, code::PROVIDER);
    assert_eq!(bad.data.map(|data| data.failure), Some(Failure::Refused));
    let listed = client.call(Request::ProviderList {}).await.unwrap();
    assert_eq!(
        listed["providers"],
        Value::Array(Vec::new()),
        "a refused key is not kept"
    );

    let connected = client
        .call(connect(ProviderKind::Openai, GOOD))
        .await
        .unwrap();
    assert!(
        !connected.to_string().contains(GOOD),
        "never answered back: {connected}"
    );
    assert_eq!(connected["provider"]["kind"], "openai");
    assert_eq!(connected["provider"]["hint"], hint(GOOD).unwrap());
    let models = client.call(Request::ModelList {}).await.unwrap();
    assert_eq!(ids(&models), ["fake-big", "fake-small"]);
    assert_eq!(models["models"][0]["default"], true);
    let listed = client.call(Request::ProviderList {}).await.unwrap();
    assert!(!listed.to_string().contains(GOOD), "{listed}");
    let id: ProviderId = serde_json::from_value(listed["providers"][0]["id"].clone()).unwrap();

    client
        .call(Request::ProviderRemove { provider: id })
        .await
        .unwrap();
    let none = client.call(Request::ModelList {}).await.unwrap();
    assert!(ids(&none).is_empty(), "its models went with it: {none}");
    let again = client
        .call(Request::ProviderRemove { provider: id })
        .await
        .unwrap_err();
    assert_eq!(again.code, code::NO_PROVIDER);
}

#[test]
fn the_person_here_connects_and_removes_a_model_account_over_each_local_transport() {
    block_on!(async {
        let tested = fresh("providers-in-process").await;
        first_run(&mut InProcess(tested.clone())).await;
        tested.shutdown().await;

        let mut client = OverJsonRpc::new(fresh("providers-json-rpc").await);
        first_run(&mut client).await;
        client.close().await;

        let mut client = OverJsonRpc::on_socket(fresh("providers-socket").await, "providers").await;
        first_run(&mut client).await;
        client.close().await;

        let token = LocalToken::new([5; 32]);
        let signin = SignIn::local(token.clone(), Box::new(fixed_clock()));
        let served = serve_with(fresh("providers-http-local").await, signin, None).await;
        let mut web = Web::new(&served, Some(token.reveal().to_owned()));
        let answered = web.call(&connect(ProviderKind::Anthropic, GOOD)).await;
        assert_eq!(
            answered["result"]["provider"]["kind"], "anthropic",
            "{answered}"
        );
        let models = web.call(&Request::ModelList {}).await;
        assert_eq!(ids(&models["result"]), ["fake-big", "fake-small"]);
        served.close().await;
    });
}

#[test]
fn a_caller_signed_in_elsewhere_may_not_connect_a_model_account() {
    block_on!(async {
        let mut client = OverHttp::new(fresh("providers-oidc").await).await;
        for request in [
            connect(ProviderKind::Openai, GOOD),
            Request::ProviderList {},
        ] {
            let refused = client.call(request).await.unwrap_err();
            assert_eq!(refused.code, code::NOT_ALLOWED, "{refused}");
        }
        let none = client.call(Request::ModelList {}).await.unwrap();
        assert!(ids(&none).is_empty(), "{none}");
        client.close().await;
    });
}

#[test]
fn a_server_whose_models_are_set_up_for_it_refuses_model_accounts() {
    block_on!(async {
        let tested = server(&folder("providers-none"), Vec::new(), Answering::Clients).await;
        let refused = tested
            .request(connect(ProviderKind::Openrouter, GOOD))
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::NOT_AVAILABLE);
        tested.shutdown().await;
    });
}
