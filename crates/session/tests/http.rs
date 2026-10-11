//! The session API over HTTP, beyond what the conformance suite checks on every transport:
//! sign-in is required and checked, the signed-in caller is recorded and the token never is,
//! keys are cached, the `Glave-Method` header must agree, and a retried turn runs once.

#[cfg(test)]
#[path = "support/model.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
#[path = "support/issuer.rs"]
mod issuer;

#[cfg(test)]
#[path = "support/web.rs"]
mod web;

#[cfg(test)]
mod http {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};

    use glave_harness::Clock;
    use glave_session::Answering;
    use glave_session::protocol::{Request, code};
    use glave_session::signin::{SignIn, SignInError, Trust, person};
    use jiff::Timestamp;
    use serde_json::{Value, json};

    use super::issuer::{AUDIENCE, Issuer, NOW, Signer};
    use super::support::*;
    use super::web::{Events, Web, serve_http};

    /// A clock the test moves.
    struct Moving(Arc<AtomicI64>);

    impl Clock for Moving {
        fn now(&self) -> Timestamp {
            Timestamp::from_second(self.0.load(Ordering::SeqCst)).unwrap()
        }
    }

    /// `claims` with `changes` merged in; a null removes a claim.
    fn with(claims: &Value, changes: &Value) -> Value {
        let mut claims = claims.clone();
        for (name, value) in changes.as_object().unwrap() {
            if value.is_null() {
                claims.as_object_mut().unwrap().remove(name);
            } else {
                claims[name] = value.clone();
            }
        }
        claims
    }

    #[test]
    fn requests_without_a_valid_sign_in_are_refused() {
        block_on(async {
            let issuer = Issuer::start().await;
            let signer = Signer::es256("key-1");
            issuer.publish(&signer);
            let stranger = Issuer::start().await;
            let theirs = Signer::es256("key-1");
            stranger.publish(&theirs);
            let unpublished = Signer::es256("key-1");
            let dir = folder("http-refused");
            let served =
                serve_http(server(&dir, Vec::new(), Answering::Clients).await, &issuer).await;
            let good = issuer.claims("ada");
            let refused = [
                ("no token", None),
                ("not a JWT", Some("not-a-jwt".to_owned())),
                (
                    "another audience",
                    Some(signer.sign(&with(&good, &json!({"aud": "other"})))),
                ),
                (
                    "expired",
                    Some(signer.sign(&with(&good, &json!({"exp": NOW - 120})))),
                ),
                (
                    "not yet valid",
                    Some(signer.sign(&with(&good, &json!({"nbf": NOW + 120})))),
                ),
                (
                    "no subject",
                    Some(signer.sign(&with(&good, &json!({"sub": null})))),
                ),
                (
                    "no expiry",
                    Some(signer.sign(&with(&good, &json!({"exp": null})))),
                ),
                (
                    "untrusted issuer",
                    Some(theirs.sign(&stranger.claims("ada"))),
                ),
                ("wrong key", Some(unpublished.sign(&good))),
                (
                    "a shared secret",
                    Some(signer.sign_as(&json!({"alg": "HS256", "kid": "key-1"}), &good)),
                ),
                (
                    "unsigned",
                    Some(signer.sign_as(&json!({"alg": "none"}), &good)),
                ),
            ];
            for (case, token) in refused {
                let mut web = Web::new(&served, token.clone());
                let (message, _) = web.message(&Request::ScheduleList {});
                let answered = web.post(&message).await;
                assert_eq!(answered.status, 401, "{case}");
                assert_eq!(
                    answered.www_authenticate.as_deref(),
                    Some("Bearer"),
                    "{case}"
                );
                let error = &answered.messages[0]["error"];
                assert_eq!(error["code"], code::NO_SIGN_IN, "{case}: {error}");
                if let Some(token) = token {
                    assert!(!answered.messages[0].to_string().contains(&token), "{case}");
                }
            }
            // A little clock skew is forgiven, and nothing was started by the refused.
            let late = signer.sign(&with(&good, &json!({"exp": NOW - 30})));
            let mut web = Web::new(&served, Some(late));
            let listed = web
                .call(&Request::ThreadList {
                    space: None,
                    limit: None,
                    show: None,
                })
                .await;
            assert_eq!(listed["result"]["threads"], json!([]), "{listed}");
            served.close().await;
        });
    }

    #[test]
    fn rs256_and_es256_tokens_both_sign_in() {
        block_on(async {
            let issuer = Issuer::start().await;
            let (ec, rsa) = (Signer::es256("ec"), Signer::rs256("rsa"));
            issuer.publish(&ec);
            issuer.publish(&rsa);
            let clock = Box::new(Moving(Arc::new(AtomicI64::new(NOW))));
            let trust = Trust {
                issuers: vec![issuer.url.clone()],
                audience: AUDIENCE.into(),
            };
            let signin = SignIn::new(trust, clock).unwrap();
            for signer in [&ec, &rsa] {
                let caller = signin
                    .verify(&signer.sign(&issuer.claims("ada")))
                    .await
                    .unwrap();
                assert_eq!(caller.person, Some(person(&issuer.url, "ada").unwrap()));
                let auth = caller.auth.unwrap();
                assert_eq!(auth.subject.as_deref(), Some("ada"));
                assert_eq!(auth.session.as_deref(), Some("session-1"));
            }
            assert_ne!(
                person(&issuer.url, "ada").unwrap(),
                person(&issuer.url, "bo").unwrap()
            );
            assert_ne!(
                person(&issuer.url, "ada").unwrap(),
                person("https://x", "ada").unwrap()
            );
        });
    }

    #[test]
    fn keys_are_cached_and_fetched_again_for_a_new_key_at_most_every_half_minute() {
        block_on(async {
            let issuer = Issuer::start().await;
            let first = Signer::es256("key-1");
            issuer.publish(&first);
            let now = Arc::new(AtomicI64::new(NOW));
            let trust = Trust {
                issuers: vec![issuer.url.clone()],
                audience: AUDIENCE.into(),
            };
            let signin = SignIn::new(trust, Box::new(Moving(Arc::clone(&now)))).unwrap();
            let claims = issuer.claims("ada");
            for _ in 0..3 {
                signin.verify(&first.sign(&claims)).await.unwrap();
            }
            assert_eq!(issuer.fetches(), 1);
            // The issuer rotates in a new key.
            let second = Signer::es256("key-2");
            issuer.publish(&second);
            let early = signin.verify(&second.sign(&claims)).await;
            assert!(
                matches!(early, Err(SignInError::UnknownKey { .. })),
                "{early:?}"
            );
            assert_eq!(issuer.fetches(), 1, "too soon to fetch again");
            now.fetch_add(31, Ordering::SeqCst);
            signin.verify(&second.sign(&claims)).await.unwrap();
            assert_eq!(issuer.fetches(), 2);
            let made_up = Signer::es256("key-3");
            assert!(signin.verify(&made_up.sign(&claims)).await.is_err());
            assert_eq!(issuer.fetches(), 2, "a made-up key id fetches nothing");
        });
    }

    #[test]
    fn an_insecure_issuer_or_none_at_all_is_refused_at_the_start() {
        let trust = |issuers: &[&str]| Trust {
            issuers: issuers.iter().map(|issuer| (*issuer).to_owned()).collect(),
            audience: AUDIENCE.into(),
        };
        let clock = || Box::new(Moving(Arc::new(AtomicI64::new(NOW))));
        assert!(matches!(
            SignIn::new(trust(&[]), clock()),
            Err(SignInError::NoIssuers)
        ));
        assert!(matches!(
            SignIn::new(trust(&["http://id.example.com"]), clock()),
            Err(SignInError::Insecure(_))
        ));
        assert!(SignIn::new(trust(&["https://id.example.com"]), clock()).is_ok());
    }

    /// The events of `thread`, as the server lists them.
    async fn events(web: &mut Web, thread: &Value) -> Vec<Value> {
        let thread = serde_json::from_value(thread.clone()).unwrap();
        let request = Request::ThreadEvents {
            thread,
            after: None,
            limit: None,
        };
        let listed = web.call(&request).await;
        listed["result"]["events"].as_array().unwrap().clone()
    }

    fn of_type<'a>(events: &'a [Value], kind: &str) -> Vec<&'a Value> {
        events
            .iter()
            .filter(|event| event["type"] == kind)
            .collect()
    }

    #[test]
    fn the_signed_in_caller_is_recorded_and_the_token_never_is() {
        block_on(async {
            let issuer = Issuer::start().await;
            let signer = Signer::es256("key-1");
            issuer.publish(&signer);
            let dir = folder("http-recorded");
            let answers = vec![
                answer(None, Some(("bash", json!({"command": "touch ran.txt"})))),
                answer(Some("Done."), None),
            ];
            let served = serve_http(server(&dir, answers, Answering::Clients).await, &issuer).await;
            let token = signer.sign(&issuer.claims("ada"));
            let mut web = Web::new(&served, Some(token.clone()));
            let started = web
                .call(&Request::ThreadStart {
                    space: None,
                    background: false,
                    worktree: None,
                })
                .await;
            let thread = started["result"]["thread"]["id"].clone();
            let start = json!({"jsonrpc": "2.0", "id": 99, "method": "turn/start",
                "params": {"thread": thread, "prompt": "touch it"}});
            let mut stream = Events::new(web.open(&start, Some("turn/start")).await);
            while let Some(message) = stream.next().await {
                if message["id"] == 99 {
                    assert_eq!(
                        message["result"]["turn"]["outcome"], "completed",
                        "{message}"
                    );
                    break;
                }
                let event = &message["params"]["event"];
                if event["type"] == "approval.requested" {
                    let decide = json!({"jsonrpc": "2.0", "id": 100, "method": "approval/decide",
                        "params": {"thread": thread, "approval": event["data"]["call_id"],
                            "decision": "allow_once"}});
                    web.post(&decide).await;
                }
            }
            assert!(dir.join("ran.txt").exists());
            let ada = json!({"kind": "person", "id": person(&issuer.url, "ada").unwrap()});
            let events = events(&mut web, &thread).await;
            // The thread is ada's work: she is its `for`, so nothing else names her.
            let started = &of_type(&events, "thread.started")[0]["data"];
            assert_eq!(started["for"], ada);
            assert_eq!(started["origin"]["by"], Value::Null);
            assert_eq!(
                of_type(&events, "turn.started")[0]["data"]["auth"],
                json!({"method": "oidc", "issuer": issuer.url, "subject": "ada",
                    "session": "session-1"})
            );
            let user = of_type(&events, "message.added")
                .into_iter()
                .find(|event| event["data"]["role"] == "user")
                .unwrap();
            assert_eq!(user["data"]["author"], Value::Null);
            assert_eq!(
                of_type(&events, "approval.decided")[0]["data"]["by"],
                Value::Null
            );
            for event in &events {
                assert!(!event.to_string().contains(&token), "{event}");
            }
            served.close().await;
        });
    }

    #[test]
    fn the_method_header_must_agree_and_only_post_to_the_root_is_served() {
        block_on(async {
            let issuer = Issuer::start().await;
            let signer = Signer::es256("key-1");
            issuer.publish(&signer);
            let dir = folder("http-header");
            let served =
                serve_http(server(&dir, Vec::new(), Answering::Clients).await, &issuer).await;
            let mut web = Web::new(&served, Some(signer.sign(&issuer.claims("ada"))));
            let (message, _) = web.message(&Request::ThreadStart {
                space: None,
                background: false,
                worktree: None,
            });
            let mismatched = web.open(&message, Some("turn/start")).await;
            let answer: Value =
                serde_json::from_str(&Events::new(mismatched).rest().await).unwrap();
            assert_eq!(answer["error"]["code"], -32600, "{answer}");
            let agreed = web.post(&message).await;
            assert_eq!(agreed.content_type, "application/json");
            assert!(agreed.messages[0]["result"]["thread"]["id"].is_string());
            let http = reqwest::Client::new();
            let got = http.get(&served.url).send().await.unwrap();
            assert_eq!(got.status().as_u16(), 405);
            let elsewhere = format!("{}rpc", served.url);
            let posted = http.post(elsewhere).body("{}").send().await.unwrap();
            assert_eq!(posted.status().as_u16(), 404);
            served.close().await;
        });
    }

    #[test]
    fn a_retried_turn_start_returns_the_turn_already_started() {
        block_on(async {
            let issuer = Issuer::start().await;
            let signer = Signer::es256("key-1");
            issuer.publish(&signer);
            let dir = folder("http-retry");
            let answers = vec![answer(Some("Once."), None), answer(Some("Twice."), None)];
            let served = serve_http(server(&dir, answers, Answering::Clients).await, &issuer).await;
            let mut web = Web::new(&served, Some(signer.sign(&issuer.claims("ada"))));
            let started = web
                .call(&Request::ThreadStart {
                    space: None,
                    background: false,
                    worktree: None,
                })
                .await;
            let thread = started["result"]["thread"]["id"].clone();
            let start = json!({"jsonrpc": "2.0", "id": 1, "method": "turn/start",
                "params": {"thread": thread, "prompt": "hi",
                    "_meta": {"idempotency_key": "attempt-1"}}});
            let first = web.post(&start).await;
            let retried = web.post(&start).await;
            assert_eq!(retried.content_type, "text/event-stream");
            assert_eq!(
                retried.messages.len(),
                1,
                "only the response: the turn had ended"
            );
            assert_eq!(retried.messages.last(), first.messages.last());
            let events = events(&mut web, &thread).await;
            assert_eq!(of_type(&events, "turn.started").len(), 1);
            served.close().await;
        });
    }
}
