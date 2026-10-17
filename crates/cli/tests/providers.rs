//! Model accounts, through the real binary over stdio with a fresh home: a first run with no
//! model connects an account, whose key is checked against the server's model list, kept
//! sealed, never shown, logged or answered back, and used for the next turn at once; it is
//! still there after a restart; settings that choose a model stay the default; and removing
//! the account takes its models away.

#[cfg(test)]
#[path = "support/binary.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
#[path = "support/served.rs"]
mod served;

#[cfg(test)]
mod accounts {
    use glave_ports::testing::FAKE_SECRET;
    use glave_provider_openai::fake::{FakeServer, Reply};
    use serde_json::{Value, json};

    use super::served::*;
    use super::support::*;

    /// The key the person pastes, which ends `wxyz`.
    fn key() -> String {
        format!("{FAKE_SECRET}-wxyz")
    }

    /// The fake server's model list: two models, the newer second.
    fn listing() -> Reply {
        Reply::Status(
            200,
            json!({"object": "list", "data": [
                {"id": "fake-old", "created": 1},
                {"id": "fake-new", "created": 2},
            ]})
            .to_string(),
        )
    }

    fn connect(url: &str, key: &str) -> Value {
        json!({"kind": "compatible", "url": url, "key": key})
    }

    #[test]
    fn a_first_run_connects_an_account_and_its_models_answer_at_once() {
        let key = key();
        let place = place("accounts-first-run");
        let fake =
            FakeServer::start(vec![listing(), Reply::text("Hello from the account.")]).unwrap();
        let mut served = Served::start(&place, &fake, &[]);
        assert!(served.models().is_empty(), "no model is set up yet");

        let connected = served.call("provider/connect", &connect(fake.url(), &key));
        let account = &connected["result"]["provider"];
        assert_eq!(account["kind"], "compatible", "{connected}");
        assert_eq!(account["hint"], "wxyz");
        assert_eq!(account["models"], json!(["fake-new", "fake-old"]));
        assert_eq!(served.models(), ["fake-new", "fake-old"]);

        let thread = served.call("thread/start", &json!({}))["result"]["thread"]["id"].clone();
        let turn = served.call("turn/start", &json!({"thread": thread, "prompt": "hi"}));
        assert_eq!(turn["result"]["turn"]["outcome"], "completed", "{turn}");
        let received = fake.received();
        assert_eq!(received[0].line, "GET /v1/models");
        assert_eq!(received[1].body["model"], "fake-new");
        let bearer = format!("Bearer {key}");
        assert!(
            received
                .iter()
                .all(|r| r.authorization.as_deref() == Some(bearer.as_str()))
        );

        let listed = served.call("provider/list", &json!({}));
        assert_eq!(listed["result"]["providers"][0]["hint"], "wxyz");
        let said = served.end();
        assert!(
            !said.contains(&key),
            "the key is never answered back or logged: {said}"
        );
        assert!(
            !holds(&place.home, &key),
            "the key is sealed at rest, and in no log"
        );

        // A restart finds the account.
        let mut served = Served::start(&place, &fake, &[]);
        assert_eq!(served.models(), ["fake-new", "fake-old"]);
        let listed = served.call("provider/list", &json!({}));
        let id = listed["result"]["providers"][0]["id"].clone();
        let removed = served.call("provider/remove", &json!({"provider": id}));
        assert_eq!(removed["result"], json!({}), "{removed}");
        assert!(served.models().is_empty(), "its models went with it");
        let again = served.call("provider/remove", &json!({"provider": id}));
        assert_eq!(again["error"]["code"], -32023);
        served.end();
    }

    #[test]
    fn a_key_the_server_refuses_is_not_kept_and_says_what_to_do() {
        let key = key();
        let place = place("accounts-refused");
        let refused = r#"{"error":{"message":"Incorrect API key provided"}}"#;
        let fake = FakeServer::start(vec![Reply::Status(401, refused.into())]).unwrap();
        let mut served = Served::start(&place, &fake, &[]);
        let answered = served.call("provider/connect", &connect(fake.url(), &key));
        assert_eq!(answered["error"]["code"], -32020, "{answered}");
        assert_eq!(answered["error"]["data"]["failure"], "refused");
        let message = answered["error"]["message"].as_str().unwrap();
        assert!(message.contains("did not accept that key"), "{message}");
        let listed = served.call("provider/list", &json!({}));
        assert_eq!(listed["result"]["providers"], json!([]));
        let far = served.call(
            "provider/connect",
            &connect("http://models.example.com/v1", &key),
        );
        assert_eq!(
            far["error"]["code"], -32602,
            "plain http only on this computer"
        );
        let said = served.end();
        assert!(!said.contains(&key), "{said}");
    }

    #[test]
    fn a_model_the_settings_choose_stays_the_default_beside_an_account() {
        let key = key();
        let place = place("accounts-precedence");
        let fake = FakeServer::start(vec![listing()]).unwrap();
        let mut served = Served::start(&place, &fake, &["--model", "chosen"]);
        assert_eq!(served.models(), ["chosen"]);
        let connected = served.call("provider/connect", &connect(fake.url(), &key));
        assert!(connected.get("result").is_some(), "{connected}");
        assert_eq!(served.models(), ["chosen", "fake-new", "fake-old"]);
        let listed = served.call("model/list", &json!({}));
        assert_eq!(listed["result"]["models"][0]["default"], true);
        served.end();
    }
}
