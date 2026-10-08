//! The client over real HTTP, against the fake server on 127.0.0.1: streaming, tool calls,
//! keys, and every way a provider can fail.

#[cfg(test)]
mod over_http {
    use glave_event::{Arguments, Part, StopReason};
    use glave_ports::{Message, ModelProvider, ModelRequest, ProviderError};
    use glave_provider_openai::fake::{FakeServer, Reply};
    use glave_provider_openai::{Config, IDLE_TIMEOUT, OpenAiCompatible};
    use serde_json::json;

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn provider(base_url: &str, key: Option<&str>) -> OpenAiCompatible {
        OpenAiCompatible::new(Config {
            base_url: base_url.to_owned(),
            api_key: key.map(str::to_owned),
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
            max_output_tokens: None,
        }
    }

    fn ask(
        provider: &OpenAiCompatible,
    ) -> (Result<glave_ports::Completion, ProviderError>, String) {
        let mut live = String::new();
        let result = block_on(provider.complete(&request(), &mut |delta| {
            if let glave_ports::Delta::Text(text) = delta {
                live.push_str(&text);
            }
        }));
        (result, live)
    }

    #[test]
    fn a_streamed_answer_arrives_live_and_whole() {
        let server = FakeServer::start(vec![Reply::text("hi there")]).unwrap();
        let (result, live) = ask(&provider(server.url(), Some("sk-test")));
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
        assert_eq!(received[0].authorization.as_deref(), Some("Bearer sk-test"));
        assert_eq!(received[0].body["model"], "fake-model");
        assert_eq!(received[0].body["stream"], true);
    }

    #[test]
    fn a_server_that_sends_nothing_times_out() {
        let server = FakeServer::start(vec![Reply::Stall]).unwrap();
        let quick = OpenAiCompatible::new(Config {
            base_url: server.url().to_owned(),
            api_key: None,
            provider: "fake".into(),
            idle_timeout: std::time::Duration::from_millis(200),
        })
        .unwrap();
        let error = block_on(quick.complete(&request(), &mut |_| {})).unwrap_err();
        assert_eq!(error, ProviderError::Timeout { seconds: 0 });
        assert!(error.is_transient(), "a silence is worth one more try");
    }

    #[test]
    fn anthropic_models_through_openrouter_ask_for_caching() {
        let server = FakeServer::start(vec![Reply::text("ok"), Reply::text("ok")]).unwrap();
        let openrouter = OpenAiCompatible::new(Config {
            base_url: server.url().to_owned(),
            api_key: None,
            provider: "openrouter".into(),
            idle_timeout: IDLE_TIMEOUT,
        })
        .unwrap();
        let mut anthropic = request();
        anthropic.model = "anthropic/claude-haiku-5.5".into();
        block_on(openrouter.complete(&anthropic, &mut |_| {})).unwrap();
        block_on(openrouter.complete(&request(), &mut |_| {})).unwrap();
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
    }

    #[test]
    fn an_unreachable_provider_is_a_transport_error() {
        // Bind and release a port, so nothing is listening there.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let (result, _) = ask(&provider(&format!("http://127.0.0.1:{port}/v1"), None));
        match result {
            Err(ProviderError::Transport(cause)) => assert!(
                cause.to_ascii_lowercase().contains("refused"),
                "names what happened, not just that a request failed: {cause}"
            ),
            other => panic!("expected a transport error, got {other:?}"),
        }
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
}
