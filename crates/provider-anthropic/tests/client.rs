//! The client over real HTTP, against the fake server on 127.0.0.1: streaming, tool calls,
//! headers, and every way a provider can fail.

#[cfg(test)]
mod over_http {
    use glave_event::{Arguments, Part, StopReason};
    use glave_ports::{Failure, Message, ModelProvider, ModelRequest, ProviderError, ToolChoice};
    use glave_provider_anthropic::fake::{FakeServer, Reply};
    use glave_provider_anthropic::{Anthropic, Api, Config, IDLE_TIMEOUT};
    use serde_json::json;

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

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
        }
    }

    fn ask(provider: &Anthropic) -> (Result<glave_ports::Completion, ProviderError>, String) {
        let mut live = String::new();
        let result = block_on(provider.complete(&request(), &mut |delta| {
            if let glave_ports::Delta::Text(text) = delta {
                live.push_str(&text);
            }
        }));
        (result, live)
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
        let (result, live) = ask(&provider(server.url(), Some("sk-ant-test")));
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
        assert_eq!(received[0].api_key.as_deref(), Some("sk-ant-test"));
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
        let provider = provider(server.url(), Some("sk-ant-wrong"));
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
    fn a_server_that_sends_nothing_times_out() {
        let server = FakeServer::start(vec![Reply::Stall]).unwrap();
        let quick = Anthropic::new(Config {
            base_url: server.url().to_owned(),
            api: Api::Messages,
            auth: glave_ports::auth::Auth::None,
            provider: "anthropic".into(),
            idle_timeout: std::time::Duration::from_millis(200),
        })
        .unwrap();
        let error = block_on(quick.complete(&request(), &mut |_| {})).unwrap_err();
        assert_eq!(error, ProviderError::Timeout { seconds: 0 });
        assert!(error.is_transient(), "a silence is worth one more try");
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
}
