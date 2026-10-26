//! The client over real HTTP, against the fake server on 127.0.0.1: streaming, tool calls,
//! keys, and every way a provider can fail.

#[cfg(test)]
mod over_http {
    use glave_event::{Arguments, Part, StopReason};
    use glave_ports::testing::fake_secret;
    use glave_ports::{
        Failure, Message, ModelProvider, ModelRequest, ProviderError, ToolChoice, block_on,
    };
    use glave_provider_openai::fake::{FakeServer, Reply};
    use glave_provider_openai::{Config, IDLE_TIMEOUT, OpenAiCompatible};
    use serde_json::json;

    fn provider(base_url: &str, key: Option<&str>) -> OpenAiCompatible {
        OpenAiCompatible::new(Config {
            base_url: base_url.to_owned(),
            auth: glave_ports::auth::Auth::bearer(key.map(str::to_owned)),
            provider: "fake".into(),
            idle_timeout: IDLE_TIMEOUT,
        })
        .unwrap()
    }

    fn request() -> ModelRequest {
        ModelRequest {
            model: "fake-model".into(),
            system: "Be brief.".into(),
            messages: vec![Message::User {
                parts: vec![Part::Text {
                    text: "hello".into(),
                }],
            }],
            tools: Vec::new(),
            tool_choice: ToolChoice::Auto,
            max_output_tokens: None,
            purpose: glave_ports::Purpose::Main,
            threads: Vec::new(),
            files: Vec::new(),
            effort: None,
        }
    }

    fn ask(
        provider: &OpenAiCompatible,
    ) -> (Result<glave_ports::Completion, ProviderError>, String) {
        let mut live = String::new();
        let result = block_on!(provider.complete(&request(), &mut |delta| {
            if let glave_ports::Delta::Text(text) = delta {
                live.push_str(&text);
            }
        }));
        (result, live)
    }

    #[test]
    fn a_streamed_answer_arrives_live_and_whole() {
        let server = FakeServer::start(vec![Reply::text("hi there")]).unwrap();
        let key = fake_secret("sk-");
        let (result, live) = ask(&provider(server.url(), Some(&key)));
        let completion = result.unwrap();
        assert_eq!(live, "hi there");
        assert_eq!(
            completion.parts,
            vec![Part::Text {
                text: "hi there".into()
            }]
        );
        assert_eq!(completion.stop, StopReason::EndTurn);
        assert_eq!(completion.usage.counts().input, 100);
        let received = server.received();
        assert_eq!(received.len(), 1);
        let bearer = format!("Bearer {key}");
        assert_eq!(received[0].authorization.as_deref(), Some(bearer.as_str()));
        assert_eq!(received[0].body["model"], "fake-model");
        assert_eq!(received[0].body["stream"], true);
    }

    #[test]
    fn anthropic_models_through_openrouter_ask_for_caching() {
        let server = FakeServer::start(vec![Reply::text("ok"), Reply::text("ok")]).unwrap();
        let openrouter = OpenAiCompatible::new(Config {
            base_url: server.url().to_owned(),
            auth: glave_ports::auth::Auth::None,
            provider: "openrouter".into(),
            idle_timeout: IDLE_TIMEOUT,
        })
        .unwrap();
        let mut anthropic = request();
        anthropic.model = "anthropic/claude-haiku-5.5".into();
        block_on!(openrouter.complete(&anthropic, &mut |_| {})).unwrap();
        block_on!(openrouter.complete(&request(), &mut |_| {})).unwrap();
        let received = server.received();
        assert_eq!(
            received[0].body["cache_control"],
            json!({"type": "ephemeral"})
        );
        assert!(received[1].body.get("cache_control").is_none());
    }

    #[test]
    fn no_key_means_no_authorization_header() {
        let server = FakeServer::start(vec![Reply::text("ok")]).unwrap();
        let (result, _) = ask(&provider(server.url(), None));
        assert!(result.is_ok());
        assert_eq!(server.received()[0].authorization, None);
    }

    fn with_auth(base_url: &str, auth: glave_ports::auth::Auth) -> OpenAiCompatible {
        OpenAiCompatible::new(Config {
            base_url: base_url.to_owned(),
            auth,
            provider: "fake".into(),
            idle_timeout: IDLE_TIMEOUT,
        })
        .unwrap()
    }

    #[test]
    fn a_key_goes_in_the_header_the_provider_wants() {
        use glave_ports::auth::{Auth, KeyHeader};
        let server = FakeServer::start(vec![Reply::text("ok")]).unwrap();
        let (result, _) = ask(&with_auth(
            server.url(),
            Auth::key(KeyHeader::ApiKey, "az-key"),
        ));
        assert!(result.is_ok());
        let received = &server.received()[0];
        assert_eq!(received.authorization, None);
        assert!(
            received
                .headers
                .contains(&("api-key".to_owned(), "az-key".to_owned()))
        );
    }

    /// Signs each request with its method and body length, as a cloud's signer would.
    struct Signer;

    impl glave_ports::auth::RequestSigner for Signer {
        fn authorize<'a>(
            &'a self,
            request: &'a glave_ports::auth::Outgoing<'a>,
        ) -> glave_ports::BoxFuture<
            'a,
            Result<Vec<glave_ports::auth::Header>, glave_ports::auth::AuthError>,
        > {
            Box::pin(async move {
                assert!(request.url.ends_with("/chat/completions"));
                Ok(vec![glave_ports::auth::Header {
                    name: "authorization".into(),
                    value: glave_ports::auth::Secret::new(format!(
                        "Signed {} {}",
                        request.method,
                        request.body.len()
                    )),
                }])
            })
        }

        fn describe(&self) -> String {
            "a test signer".into()
        }
    }

    /// Credentials that cannot be had.
    struct Missing;

    impl glave_ports::auth::RequestSigner for Missing {
        fn authorize<'a>(
            &'a self,
            _request: &'a glave_ports::auth::Outgoing<'a>,
        ) -> glave_ports::BoxFuture<
            'a,
            Result<Vec<glave_ports::auth::Header>, glave_ports::auth::AuthError>,
        > {
            Box::pin(async {
                Err(glave_ports::auth::AuthError::refused(
                    "the token file /var/run/token is missing",
                ))
            })
        }

        fn describe(&self) -> String {
            "a missing token".into()
        }
    }

    #[test]
    fn changing_credentials_sign_each_request_or_say_why_they_cannot() {
        use std::sync::Arc;
        let server = FakeServer::start(vec![Reply::text("ok")]).unwrap();
        let signed = with_auth(
            server.url(),
            glave_ports::auth::Auth::Changing(Arc::new(Signer)),
        );
        assert!(ask(&signed).0.is_ok());
        let authorization = server.received()[0].authorization.clone().unwrap();
        let length: usize = authorization
            .strip_prefix("Signed POST ")
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            length > 100,
            "the signer saw the whole body: {length} bytes"
        );
        let missing = with_auth(
            server.url(),
            glave_ports::auth::Auth::Changing(Arc::new(Missing)),
        );
        let error = ask(&missing).0.unwrap_err();
        assert_eq!(error.failure(), Failure::Refused);
        assert!(error.to_string().contains("token file"), "{error}");
        assert_eq!(
            server.received().len(),
            1,
            "nothing is sent without credentials"
        );
    }

    #[test]
    fn tool_calls_come_back_assembled() {
        let arguments = json!({"path": "notes.md", "content": "hi"});
        let server =
            FakeServer::start(vec![Reply::tool_call("toolu_1", "write", &arguments)]).unwrap();
        let (result, _) = ask(&provider(server.url(), None));
        let completion = result.unwrap();
        assert_eq!(completion.stop, StopReason::ToolUse);
        assert_eq!(completion.tool_calls.len(), 1);
        let call = &completion.tool_calls[0];
        assert_eq!(call.tool, "write");
        assert_eq!(call.provider_call_id.as_deref(), Some("toolu_1"));
        assert!(matches!(&call.arguments, Arguments::Json(map) if map["path"] == "notes.md"));
    }

    #[test]
    fn an_error_status_keeps_its_code_and_the_message_alone() {
        let long = "x".repeat(5_000);
        let server = FakeServer::start(vec![
            Reply::Status(
                401,
                r#"{"error":{"message":"Missing Authentication header","code":401}}"#.into(),
            ),
            Reply::Status(500, long),
            // Vertex AI's, for Gemini there.
            Reply::Status(
                404,
                r#"[{"error":{"code":404,"message":"Publisher Model gemini-nope not found.","status":"NOT_FOUND"}}]"#.into(),
            ),
        ])
        .unwrap();
        let provider = provider(server.url(), None);
        match ask(&provider).0 {
            Err(ProviderError::Http { status, body }) => {
                assert_eq!(status, 401);
                assert_eq!(body, "Missing Authentication header", "not raw JSON");
            }
            other => panic!("expected HTTP 401, got {other:?}"),
        }
        match ask(&provider).0 {
            Err(ProviderError::Http { status, body }) => {
                assert_eq!(status, 500);
                assert_eq!(body.chars().count(), 500, "a long body is cut short");
            }
            other => panic!("expected HTTP 500, got {other:?}"),
        }
        assert_eq!(
            ask(&provider).0.unwrap_err(),
            ProviderError::Http {
                status: 404,
                body: "Publisher Model gemini-nope not found.".into()
            },
            "not raw JSON"
        );
    }

    #[test]
    fn a_key_the_server_repeats_never_shows_in_a_listing_s_error() {
        let key = fake_secret("sk-");
        let said = format!(r#"{{"error":{{"message":"Incorrect API key provided: {key}."}}}}"#);
        let server = FakeServer::start(vec![Reply::Status(401, said)]).unwrap();
        let provider = provider(server.url(), Some(&key));
        let error = block_on!(provider.get("models")).unwrap_err();
        assert!(!format!("{error} {error:?}").contains(&key), "{error:?}");
        assert!(
            error
                .to_string()
                .contains("Incorrect API key provided: [redacted]."),
            "{error}"
        );
    }

    #[test]
    fn an_unreachable_provider_is_a_transport_error() {
        // Port 1: a privileged port nothing listens on, and never one of the ephemeral ports the
        // other tests' fake servers take. A port bound and released here could be taken by one
        // of them before the request, which then answers it.
        let (result, _) = ask(&provider("http://127.0.0.1:1/v1", None));
        match result {
            Err(ProviderError::Transport(cause)) => assert!(
                cause.to_ascii_lowercase().contains("refused"),
                "names what happened, not just that a request failed: {cause}"
            ),
            other => panic!("expected a transport error, got {other:?}"),
        }
    }

    #[test]
    fn a_connection_that_never_opens_is_the_provider_unreached_not_silent() {
        // A listener that never accepts: the system completes the TCP handshake and nothing
        // answers TLS's, so the connection never opens. The paused clock jumps to the limit.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let provider = provider(
            &format!("https://{}/v1", listener.local_addr().unwrap()),
            None,
        );
        let result = block_on!(async {
            tokio::time::pause();
            provider.complete(&request(), &mut |_| {}).await
        });
        let error = result.unwrap_err();
        assert_eq!(error.failure(), Failure::Unreachable, "{error:?}");
        assert!(
            error
                .to_string()
                .ends_with("no connection to 127.0.0.1 within 10 seconds"),
            "says what was waited for and how long: {error}"
        );
    }

    #[test]
    fn a_stream_that_never_finishes_is_not_a_finished_answer() {
        let server = FakeServer::start(vec![Reply::Stream(vec![json!(
            {"choices": [{"index": 0, "delta": {"content": "half an ans"}}]}
        )])])
        .unwrap();
        let (result, live) = ask(&provider(server.url(), None));
        assert_eq!(live, "half an ans");
        let completion = result.unwrap();
        assert_eq!(completion.stop, StopReason::Interrupted);
        assert!(completion.usage.estimated(), "no usage was reported");
    }

    #[test]
    fn a_malformed_chunk_is_a_protocol_error() {
        let server = FakeServer::start(vec![Reply::Stream(vec![json!("not a chunk")])]).unwrap();
        let (result, _) = ask(&provider(server.url(), None));
        assert!(
            matches!(result, Err(ProviderError::Protocol(_))),
            "{result:?}"
        );
    }

    #[test]
    fn a_listing_is_asked_with_the_key_and_read_as_json() {
        let listed = r#"{"object":"list","data":[{"id":"gpt-test","created":1}]}"#;
        let server = FakeServer::start(vec![
            Reply::Status(200, listed.into()),
            Reply::Status(401, r#"{"error":{"message":"Incorrect API key"}}"#.into()),
        ])
        .unwrap();
        let key = fake_secret("sk-");
        let provider = provider(server.url(), Some(&key));
        let models = block_on!(provider.get("models")).unwrap();
        assert_eq!(models["data"][0]["id"], "gpt-test");
        let refused = block_on!(provider.get("/models")).unwrap_err();
        assert_eq!(refused.failure(), Failure::Refused);
        let received = server.received();
        assert_eq!(received[0].line, "GET /v1/models");
        let bearer = format!("Bearer {key}");
        assert_eq!(received[0].authorization.as_deref(), Some(bearer.as_str()));
        assert!(!format!("{refused:?}").contains(&key));
    }
}

#[cfg(test)]
mod conformance {
    use glave_ports::conformance::{Script, model_provider};
    use glave_ports::{auth::Auth, block_on};
    use glave_provider_openai::fake::{FakeServer, Reply};
    use glave_provider_openai::{Config, IDLE_TIMEOUT, OpenAiCompatible};
    use serde_json::{Value, json};

    fn delta(delta: &Value) -> Value {
        json!({"id": "gen-fake", "model": "fake-model", "choices": [{"index": 0, "delta": delta}]})
    }

    fn text(pieces: &[&str]) -> Vec<Value> {
        pieces
            .iter()
            .map(|piece| delta(&json!({"content": piece})))
            .collect()
    }

    fn finish(usage: bool) -> Value {
        let mut last = json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]});
        if usage {
            last["usage"] = json!({"prompt_tokens": 100, "completion_tokens": 10});
        }
        last
    }

    fn mystery() -> Value {
        json!({"error": {"message": "something nobody named", "code": "mystery"}})
    }

    /// The suite's scripts in OpenAI's chat completions stream.
    fn reply(script: Script, key: &str) -> Reply {
        match script {
            Script::Answer {
                reasoning,
                text: said,
            } => {
                let mut chunks: Vec<Value> = reasoning
                    .iter()
                    .map(|piece| delta(&json!({"reasoning_content": piece})))
                    .collect();
                chunks.extend(text(said));
                chunks.push(finish(true));
                Reply::Stream(chunks)
            }
            Script::Unmetered { text: said } => {
                let mut chunks = text(said);
                chunks.push(finish(false));
                Reply::Stream(chunks)
            }
            Script::ErrorFirst => {
                Reply::Stream(vec![delta(&json!({"role": "assistant"})), mystery()])
            }
            Script::ErrorAfter { text: said } => {
                let mut chunks = text(&[said]);
                chunks.push(mystery());
                Reply::Stream(chunks)
            }
            Script::Silence => Reply::Stall,
            Script::Refusal => Reply::Status(
                401,
                json!({"error": {"message": format!("Incorrect API key provided: {key}.")}})
                    .to_string(),
            ),
        }
    }

    #[test]
    fn the_provider_passes_the_model_provider_suite() {
        let verdict = block_on!(model_provider(|script, key| {
            let server = FakeServer::start(vec![reply(script, key)]).map_err(|e| e.to_string())?;
            let provider = OpenAiCompatible::new(Config {
                base_url: server.url().to_owned(),
                auth: Auth::bearer(Some(key.to_owned())),
                provider: "fake".into(),
                idle_timeout: IDLE_TIMEOUT,
            })
            .map_err(|e| e.to_string())?;
            Ok((provider, server))
        }));
        assert_eq!(verdict, Ok(()));
    }
}
