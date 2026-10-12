//! Local sign-in over HTTP: a browser on this computer signs in with the server's local token
//! and is the local person, recorded as signed in `local`; without the token, or with another,
//! it is refused; and the token is refused from any other computer.

#[cfg(test)]
#[path = "support/model.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
#[path = "support/issuer.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod issuer;

#[cfg(test)]
#[path = "support/web.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod web;

#[cfg(test)]
mod local {
    use std::net::IpAddr;

    use glave_event::AuthMethod;
    use glave_session::Answering;
    use glave_session::protocol::{Meta, Request, code};
    use glave_session::signin::{LocalToken, SignIn, SignInError};
    use serde_json::{Value, json};

    use super::issuer::Fixed;
    use super::support::*;
    use super::web::{Web, serve_with};

    #[test]
    fn a_browser_here_signs_in_with_the_local_token_as_the_local_person() {
        block_on(async {
            let dir = folder("http-local");
            let answers = vec![answer(Some("Hello."), None)];
            let token = LocalToken::new([7; 32]);
            let signin = SignIn::local(token.clone(), Box::new(Fixed));
            let served = serve_with(server(&dir, answers, Answering::Clients).await, signin).await;

            let mut web = Web::new(&served, Some(token.reveal().to_owned()));
            let started = web
                .call(&Request::ThreadStart {
                    space: None,
                    worktree: None,
                    background: false,
                    computer: None,
                    model: None,
                    approval: None,
                })
                .await;
            let thread = started["result"]["thread"]["id"].clone();
            assert!(thread.is_string(), "{started}");
            let (message, _) = web.message(&Request::TurnStart {
                thread: serde_json::from_value(thread.clone()).unwrap(),
                prompt: "hi".into(),
                background: false,
                budget: None,
                meta: Meta::default(),
                model: None,
                approval: None,
            });
            let answered = web.post(&message).await;
            assert_eq!(answered.status, 200);
            let events = web
                .call(&Request::ThreadEvents {
                    thread: serde_json::from_value(thread).unwrap(),
                    after: None,
                    limit: None,
                })
                .await;
            let turn = events["result"]["events"]
                .as_array()
                .unwrap()
                .iter()
                .find(|event| event["type"] == "turn.started")
                .cloned()
                .unwrap();
            assert_eq!(turn["data"]["auth"], json!({"method": "local"}));
            let recorded = serde_json::to_string(&events).unwrap();
            assert!(
                !recorded.contains(token.reveal()),
                "the token is never stored"
            );

            for (case, presented) in [
                ("no token", None),
                (
                    "another server's",
                    Some(LocalToken::new([8; 32]).reveal().to_owned()),
                ),
                ("not a token", Some("glave_local_".to_owned())),
                (
                    "an OIDC token, with no issuer trusted",
                    Some("e30.e30.c2ln".to_owned()),
                ),
            ] {
                let refused = Web::new(&served, presented)
                    .call(&Request::Discover {})
                    .await;
                assert_eq!(refused["error"]["code"], json!(code::NO_SIGN_IN), "{case}");
                let message: &Value = &refused["error"]["message"];
                assert!(!message.to_string().contains(token.reveal()), "{case}");
            }
            served.close().await;
        });
    }

    #[test]
    fn the_local_token_is_refused_from_another_computer() {
        block_on(async {
            let token = LocalToken::new([9; 32]);
            let signin = SignIn::local(token.clone(), Box::new(Fixed));
            let here: IpAddr = "127.0.0.1".parse().unwrap();
            let there: IpAddr = "192.168.1.20".parse().unwrap();
            let caller = signin.verify_from(token.reveal(), here).await.unwrap();
            assert_eq!(caller.person, None, "the local person");
            assert_eq!(caller.auth.map(|auth| auth.method), Some(AuthMethod::Local));
            assert!(matches!(
                signin.verify_from(token.reveal(), there).await,
                Err(SignInError::NotLoopback)
            ));
            let ipv6: IpAddr = "::1".parse().unwrap();
            assert!(signin.verify_from(token.reveal(), ipv6).await.is_ok());
        });
    }
}
