//! Model calls through the gateway service, end to end on 127.0.0.1: glave's own OpenAI and
//! Anthropic providers as the clients, a fake provider behind the gateway, sign-in by a fake
//! identity provider. Both surfaces stream; every call needs a sign-in; each caller's policy
//! and budget are their own; every call is recorded in the caller's stream; and the gateway's
//! key never reaches a caller.

#[cfg(test)]
#[path = "../../session/tests/support/issuer.rs"]
#[expect(
    dead_code,
    reason = "the session API's fake identity provider; these tests use part"
)]
mod issuer;
#[cfg(test)]
#[path = "support/world.rs"]
mod world;

#[cfg(test)]
mod surfaces {
    use glave_event::{Body, StopReason};
    use glave_gateway::{Budgets, Limit};
    use glave_ports::{Delta, Failure, Message, ModelProvider, ModelRequest, ProviderError};
    use glave_provider_anthropic::Anthropic;
    use glave_provider_openai::fake::Reply;
    use glave_provider_openai::{Config, OpenAiCompatible};
    use serde_json::{Value, json};

    use super::world::{KEY, World, block_on};

    fn ask(model: &str, text: &str) -> ModelRequest {
        ModelRequest {
            model: model.into(),
            system: "Be brief.".into(),
            messages: vec![Message::User {
                parts: vec![glave_event::Part::Text { text: text.into() }],
            }],
            tools: Vec::new(),
            tool_choice: glave_ports::ToolChoice::Auto,
            max_output_tokens: Some(20),
            purpose: glave_ports::Purpose::Main,
            threads: Vec::new(),
        }
    }

    fn openai(world: &World, token: &str) -> OpenAiCompatible {
        let mut config = Config::for_url(format!("{}/v1", world.url), Some(token.into())).unwrap();
        config.provider = "gateway".into();
        OpenAiCompatible::new(config).unwrap()
    }

    fn anthropic(world: &World, token: &str) -> Anthropic {
        Anthropic::new(glave_provider_anthropic::Config {
            base_url: world.url.clone(),
            auth: glave_ports::auth::Auth::key(glave_ports::auth::KeyHeader::XApiKey, token),
            provider: "gateway".into(),
            idle_timeout: glave_provider_anthropic::IDLE_TIMEOUT,
        })
        .unwrap()
    }

    #[test]
    fn the_openai_surface_streams_an_answer_from_the_route_s_target() {
        block_on(async {
            let world =
                World::start(vec![Reply::text("Hello there.")], Budgets::default(), false).await;
            let mut live = Vec::new();
            let completion = openai(&world, &world.token("alice"))
                .complete(&ask("main", "hi"), &mut |delta| live.push(delta))
                .await
                .unwrap();
            assert_eq!(
                live,
                [Delta::Text("Hello there.".into())],
                "streamed as it came"
            );
            assert_eq!(completion.stop, StopReason::EndTurn);
            assert_eq!(completion.usage.counts().input, 100);
            assert_eq!(completion.usage.counts().output, 10);
            let sent = world.upstream.received();
            assert_eq!(sent[0].body["model"], "big", "the main route's target");
            assert_eq!(
                sent[0].authorization.as_deref(),
                Some(&*format!("Bearer {KEY}"))
            );
            let recorded = world.recorded("alice").await;
            let [Body::ModelResponded(responded)] = recorded.as_slice() else {
                panic!("{recorded:?}");
            };
            assert_eq!(responded.route.as_deref(), Some("main"));
            assert_eq!(responded.target.as_deref(), Some("acme/big"));
            assert_eq!(responded.usage.counts().output, 10);
            assert!(
                world.recorded("bob").await.is_empty(),
                "each caller has a stream"
            );
        });
    }

    #[test]
    fn the_anthropic_surface_streams_text_and_tool_calls() {
        block_on(async {
            let replies = vec![Reply::tool_call("call_9", "ls", &json!({"path": "."}))];
            let world = World::start(replies, Budgets::default(), false).await;
            let mut request = ask("acme/big", "list the files");
            request.tools = vec![glave_ports::ToolSpec {
                name: "ls".into(),
                description: "List files.".into(),
                parameters: json!({"type": "object"}),
            }];
            let completion = anthropic(&world, &world.token("alice"))
                .complete(&request, &mut |_| {})
                .await
                .unwrap();
            assert_eq!(completion.stop, StopReason::ToolUse);
            assert_eq!(completion.tool_calls[0].tool, "ls");
            assert_eq!(
                completion.tool_calls[0].provider_call_id.as_deref(),
                Some("call_9")
            );
            assert_eq!(completion.usage.counts().input, 100);
            let sent = &world.upstream.received()[0].body;
            assert_eq!(sent["tools"][0]["function"]["name"], "ls");
            assert_eq!(sent["max_tokens"], 20);
        });
    }

    #[test]
    fn both_surfaces_answer_whole_when_no_stream_is_asked_for() {
        block_on(async {
            let replies = vec![Reply::text("One."), Reply::text("Two.")];
            let world = World::start(replies, Budgets::default(), false).await;
            let token = world.token("alice");
            let http = reqwest::Client::new();
            let chat: Value = http
                .post(format!("{}/v1/chat/completions", world.url))
                .bearer_auth(&token)
                .json(&json!({"model": "main", "messages": [{"role": "user", "content": "hi"}]}))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            assert_eq!(chat["choices"][0]["message"]["content"], "One.");
            assert_eq!(chat["usage"]["prompt_tokens"], 100);
            let message: Value = http
                .post(format!("{}/v1/messages", world.url))
                .header("x-api-key", &token)
                .json(&json!({"model": "main", "max_tokens": 9,
                              "messages": [{"role": "user", "content": "hi"}]}))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            assert_eq!(message["content"][0]["text"], "Two.");
            assert_eq!(message["stop_reason"], "end_turn");
        });
    }

    #[test]
    fn every_call_needs_a_sign_in() {
        block_on(async {
            let world = World::start(vec![Reply::text("never")], Budgets::default(), true).await;
            let http = reqwest::Client::new();
            for path in ["/v1/chat/completions", "/v1/messages", "/mcp"] {
                let bare = http
                    .post(format!("{}{path}", world.url))
                    .json(&json!({"model": "main", "messages": []}))
                    .send()
                    .await
                    .unwrap();
                assert_eq!(bare.status(), 401, "{path}");
                assert_eq!(bare.headers()["www-authenticate"], "Bearer");
                let forged = http
                    .post(format!("{}{path}", world.url))
                    .bearer_auth("not.a.token")
                    .send()
                    .await
                    .unwrap();
                assert_eq!(forged.status(), 401, "{path}");
            }
            let failed = openai(&world, "not.a.token")
                .complete(&ask("main", "hi"), &mut |_| {})
                .await
                .unwrap_err();
            assert_eq!(failed.failure(), Failure::Refused);
            assert!(world.upstream.received().is_empty(), "nothing went out");
        });
    }

    #[test]
    fn each_caller_is_held_to_their_own_allow_list() {
        block_on(async {
            let world = World::start(vec![Reply::text("Judged.")], Budgets::default(), false).await;
            let refused = openai(&world, &world.token("bob"))
                .complete(&ask("judge", "hi"), &mut |_| {})
                .await
                .unwrap_err();
            let ProviderError::Http { status, body } = &refused else {
                panic!("{refused:?}");
            };
            assert_eq!(*status, 403);
            assert!(body.contains("model acme/small is not allowed"), "{body}");
            let allowed = openai(&world, &world.token("alice"))
                .complete(&ask("judge", "hi"), &mut |_| {})
                .await
                .unwrap();
            assert_eq!(
                allowed.served, None,
                "the client sees the provider, not the gateway"
            );
            assert_eq!(world.upstream.received()[0].body["model"], "small");
        });
    }

    #[test]
    fn each_caller_has_their_own_budget_and_a_restart_does_not_reset_it() {
        block_on(async {
            let budgets = Budgets {
                person: Some(Limit::Tokens(120)),
                ..Budgets::default()
            };
            let replies = vec![Reply::text("a"), Reply::text("b"), Reply::text("c")];
            let mut world = World::start(replies, budgets, false).await;
            let alice = world.token("alice");
            openai(&world, &alice)
                .complete(&ask("main", "hi"), &mut |_| {})
                .await
                .unwrap();
            let over = openai(&world, &alice)
                .complete(&ask("main", "again"), &mut |_| {})
                .await
                .unwrap_err();
            assert!(
                matches!(&over, ProviderError::Http { status: 402, body } if body.contains("budget")),
                "{over:?}"
            );
            openai(&world, &world.token("bob"))
                .complete(&ask("main", "hi"), &mut |_| {})
                .await
                .unwrap();
            world.serve(budgets, false).await;
            let after_restart = openai(&world, &alice)
                .complete(&ask("main", "once more"), &mut |_| {})
                .await
                .unwrap_err();
            assert!(
                matches!(&after_restart, ProviderError::Http { status: 402, .. }),
                "what alice spent today is read back from her stream: {after_restart:?}"
            );
            assert_eq!(world.upstream.received().len(), 2);
        });
    }

    #[test]
    fn the_gateway_s_key_never_reaches_a_caller_or_a_notice() {
        block_on(async {
            let echoing = format!(r#"{{"error": {{"message": "bad request with key {KEY}"}}}}"#);
            let replies = vec![
                Reply::Status(400, echoing),
                Reply::Status(401, format!("key {KEY} refused")),
            ];
            let world = World::start(replies, Budgets::default(), false).await;
            let client = openai(&world, &world.token("alice"));
            let first = client
                .complete(&ask("main", "hi"), &mut |_| {})
                .await
                .unwrap_err();
            let second = client
                .complete(&ask("main", "hi"), &mut |_| {})
                .await
                .unwrap_err();
            for failed in [&first, &second] {
                assert!(!failed.to_string().contains(KEY), "{failed}");
            }
            assert!(first.to_string().contains("[redacted]"), "{first}");
            assert!(
                matches!(&second, ProviderError::Http { status: 502, .. }),
                "the gateway's own key refused is not the caller's sign-in: {second:?}"
            );
            let notices = world.notices.lock().unwrap().join("\n");
            assert!(!notices.contains(KEY), "{notices}");
        });
    }
}
