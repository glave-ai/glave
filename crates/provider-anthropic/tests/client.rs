//! The client over real HTTP, against the fake server on 127.0.0.1: streaming, tool calls,
//! headers, and every way a provider can fail.

#[cfg(test)]
mod over_http {
    use glave_event::{Arguments, Part, StopReason};
    use glave_ports::testing::fake_secret;
    use glave_ports::{
        Failure, Message, ModelProvider, ModelRequest, ProviderError, ToolChoice, block_on,
    };
    use glave_provider_anthropic::fake::{FakeServer, Reply};
    use glave_provider_anthropic::{Anthropic, Api, Config, IDLE_TIMEOUT};
    use serde_json::json;

    fn provider(base_url: &str, key: Option<&str>) -> Anthropic {
        Anthropic::new(Config {
            base_url: base_url.to_owned(),
            api: Api::Messages,
            auth: glave_ports::auth::Auth::bearer(key.map(str::to_owned))
                .in_header(glave_ports::auth::KeyHeader::XApiKey),
            provider: "anthropic".into(),
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
        }
    }

    fn ask(provider: &Anthropic) -> (Result<glave_ports::Completion, ProviderError>, String) {
        let mut live = String::new();
        let result = block_on!(provider.complete(&request(), &mut |delta| {
            if let glave_ports::Delta::Text(text) = delta {
                live.push_str(&text);
            }
        }));
        (result, live)
    }

    #[test]
    fn a_key_the_server_repeats_never_shows_in_a_listing_s_error() {
        let key = fake_secret("sk-ant-");
        let said = format!(
            r#"{{"type":"error","error":{{"type":"authentication_error","message":"invalid x-api-key: {key}"}}}}"#
        );
        let server = FakeServer::start(vec![Reply::Status(401, said)]).unwrap();
        let provider = provider(server.url(), Some(&key));
        let error = block_on!(provider.get("models")).unwrap_err();
        assert!(!format!("{error} {error:?}").contains(&key), "{error:?}");
        assert!(
            error.to_string().contains("invalid x-api-key: [redacted]"),
            "{error}"
        );
    }

    #[test]
    fn vertex_is_asked_at_stream_raw_predict_with_its_version_in_the_body() {
        let server = FakeServer::start(vec![Reply::text("hi from vertex")]).unwrap();
        let vertex = Anthropic::new(Config {
            base_url: format!("{}/v1/projects/acme-ai/locations/global/", server.url()),
            api: Api::Vertex,
            auth: glave_ports::auth::Auth::bearer(Some("ya29.token".into())),
            provider: "vertex".into(),
            idle_timeout: IDLE_TIMEOUT,
        })
        .unwrap();
        let (result, live) = ask(&vertex);
        assert_eq!(result.unwrap().stop, StopReason::EndTurn);
        assert_eq!(live, "hi from vertex");
        let received = server.received();
        assert_eq!(
            received[0].path,
            "/v1/projects/acme-ai/locations/global/publishers/anthropic/models/fake-model:streamRawPredict"
        );
        assert_eq!(received[0].version, None, "no anthropic-version header");
        assert_eq!(received[0].api_key, None, "a bearer token, not x-api-key");
        assert_eq!(received[0].body["anthropic_version"], "vertex-2023-10-16");
        assert!(received[0].body.get("model").is_none());
        assert_eq!(received[0].body["stream"], true);
        assert_eq!(received[0].body["system"], "Be brief.");
    }

    #[test]
    fn a_streamed_answer_arrives_live_and_whole() {
        let server = FakeServer::start(vec![Reply::text("hi there")]).unwrap();
        let key = fake_secret("sk-ant-");
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
        assert_eq!(completion.usage.counts().output, 10);
        assert_eq!(completion.response_id.as_deref(), Some("msg_fake"));
        let received = server.received();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].path, "/v1/messages");
        assert_eq!(received[0].api_key.as_deref(), Some(key.as_str()));
        assert_eq!(received[0].version.as_deref(), Some("2023-06-01"));
        assert_eq!(received[0].body["model"], "fake-model");
        assert_eq!(received[0].body["system"], "Be brief.");
        assert_eq!(received[0].body["stream"], true);
    }

    #[test]
    fn no_key_means_no_key_header() {
        let server = FakeServer::start(vec![Reply::text("ok")]).unwrap();
        let (result, _) = ask(&provider(server.url(), None));
        assert!(result.is_ok());
        assert_eq!(server.received()[0].api_key, None);
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
        let server = FakeServer::start(vec![
            Reply::Status(
                401,
                r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#.into(),
            ),
            Reply::Status(
                529,
                r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#.into(),
            ),
            Reply::Status(
                400,
                r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 210000 tokens > 200000 maximum"}}"#.into(),
            ),
        ])
        .unwrap();
        let provider = provider(server.url(), Some(&fake_secret("sk-ant-")));
        let refused = ask(&provider).0.unwrap_err();
        assert_eq!(
            refused,
            ProviderError::Http {
                status: 401,
                body: "invalid x-api-key".into()
            },
            "not raw JSON"
        );
        assert_eq!(refused.failure(), Failure::Refused);
        let overloaded = ask(&provider).0.unwrap_err();
        assert!(overloaded.is_transient(), "{overloaded:?}");
        assert_eq!(ask(&provider).0.unwrap_err().failure(), Failure::TooLong);
    }

    #[test]
    fn an_error_mid_stream_fails_the_request() {
        let server = FakeServer::start(vec![Reply::Stream(vec![
            json!({"type": "message_start", "message": {"id": "msg_1", "model": "fake-model", "usage": {"input_tokens": 5, "output_tokens": 1}}}),
            json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}}),
        ])])
        .unwrap();
        let error = ask(&provider(server.url(), None)).0.unwrap_err();
        assert_eq!(error.failure(), Failure::Unavailable);
    }

    #[test]
    fn an_unreachable_provider_is_a_transport_error() {
        // Bind and release a port, so nothing is listening there.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let (result, _) = ask(&provider(&format!("http://127.0.0.1:{port}"), None));
        match result {
            Err(ProviderError::Transport(cause)) => assert!(
                cause.to_ascii_lowercase().contains("refused"),
                "names what happened, not just that a request failed: {cause}"
            ),
            other => panic!("expected a transport error, got {other:?}"),
        }
    }

    #[test]
    fn a_stream_that_never_stops_is_not_a_finished_answer() {
        let server = FakeServer::start(vec![Reply::Stream(vec![
            json!({"type": "message_start", "message": {"id": "msg_1", "model": "fake-model", "usage": {"input_tokens": 5, "output_tokens": 1}}}),
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "half an ans"}}),
        ])])
        .unwrap();
        let (result, live) = ask(&provider(server.url(), None));
        assert_eq!(live, "half an ans");
        let completion = result.unwrap();
        assert_eq!(completion.stop, StopReason::Interrupted);
        assert!(completion.usage.estimated());
    }

    #[test]
    fn a_listing_is_asked_with_the_key_and_the_version() {
        let listed = r#"{"data":[{"id":"claude-test","created_at":"2026-10-01T00:00:00Z"}],"has_more":false}"#;
        let server = FakeServer::start(vec![
            Reply::Status(200, listed.into()),
            Reply::Status(401, r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#.into()),
        ])
        .unwrap();
        let key = fake_secret("sk-ant-");
        let provider = provider(server.url(), Some(&key));
        let models = block_on!(provider.get("models?limit=1000")).unwrap();
        assert_eq!(models["data"][0]["id"], "claude-test");
        let refused = block_on!(provider.get("models")).unwrap_err();
        assert_eq!(refused.failure(), Failure::Refused);
        let received = server.received();
        assert_eq!(received[0].path, "/v1/models?limit=1000");
        assert_eq!(received[0].api_key.as_deref(), Some(key.as_str()));
        assert_eq!(received[0].version.as_deref(), Some("2023-06-01"));
    }
}

#[cfg(test)]
mod conformance {
    use glave_ports::auth::{Auth, KeyHeader};
    use glave_ports::block_on;
    use glave_ports::conformance::{Script, model_provider};
    use glave_provider_anthropic::fake::{FakeServer, Reply};
    use glave_provider_anthropic::{Anthropic, Api, Config, IDLE_TIMEOUT};
    use serde_json::{Value, json};

    fn start(usage: bool) -> Value {
        let mut message = json!({"id": "msg_fake", "type": "message", "role": "assistant",
            "model": "fake-model", "content": [], "stop_reason": null});
        if usage {
            message["usage"] = json!({"input_tokens": 100, "output_tokens": 1});
        }
        json!({"type": "message_start", "message": message})
    }

    fn begin(index: u32, block: &Value) -> Value {
        json!({"type": "content_block_start", "index": index, "content_block": block})
    }

    /// The block at `index` grown by `pieces`, each a delta of `kind` under `field`.
    fn deltas(index: u32, kind: &str, field: &str, pieces: &[&str]) -> Vec<Value> {
        let delta = |piece| json!({"type": "content_block_delta", "index": index, "delta": {"type": kind, field: piece}});
        pieces.iter().map(delta).collect()
    }

    fn text(index: u32, pieces: &[&str]) -> Vec<Value> {
        let mut events = vec![begin(index, &json!({"type": "text", "text": ""}))];
        events.extend(deltas(index, "text_delta", "text", pieces));
        events
    }

    fn end(usage: bool) -> Vec<Value> {
        let mut delta = json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}});
        if usage {
            delta["usage"] = json!({"output_tokens": 10});
        }
        vec![delta, json!({"type": "message_stop"})]
    }

    fn mystery() -> Value {
        json!({"type": "error", "error": {"type": "mystery_error", "message": "something nobody named"}})
    }

    /// The suite's scripts in the Messages API's event stream.
    fn reply(script: Script, key: &str) -> Reply {
        let events = match script {
            Script::Answer {
                reasoning,
                text: said,
            } => {
                let thinking = json!({"type": "thinking", "thinking": "", "signature": ""});
                let mut events = vec![start(true), begin(0, &thinking)];
                events.extend(deltas(0, "thinking_delta", "thinking", reasoning));
                events.extend(deltas(0, "signature_delta", "signature", &["c2ln"]));
                events.extend(text(1, said));
                events.extend(end(true));
                events
            }
            Script::Unmetered { text: said } => {
                let mut events = vec![start(false)];
                events.extend(text(0, said));
                events.extend(end(false));
                events
            }
            Script::ErrorFirst => vec![start(true), mystery()],
            Script::ErrorAfter { text: said } => {
                let mut events = vec![start(true)];
                events.extend(text(0, &[said]));
                events.push(mystery());
                events
            }
            Script::Silence => return Reply::Stall,
            Script::Refusal => {
                let message = format!("invalid x-api-key: {key}");
                let body = json!({"type": "error", "error": {"type": "authentication_error", "message": message}});
                return Reply::Status(401, body.to_string());
            }
        };
        Reply::Stream(events)
    }

    #[test]
    fn the_provider_passes_the_model_provider_suite() {
        let verdict = block_on!(model_provider(|script, key| {
            let server = FakeServer::start(vec![reply(script, key)]).map_err(|e| e.to_string())?;
            let provider = Anthropic::new(Config {
                base_url: server.url().to_owned(),
                api: Api::Messages,
                auth: Auth::key(KeyHeader::XApiKey, key),
                provider: "anthropic".into(),
                idle_timeout: IDLE_TIMEOUT,
            })
            .map_err(|e| e.to_string())?;
            Ok((provider, server))
        }));
        assert_eq!(verdict, Ok(()));
    }
}
