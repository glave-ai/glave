//! Claude on Bedrock: Anthropic's Messages API at Bedrock's `/anthropic` route, through
//! `glave-provider-anthropic`, signed with SigV4 or sent a Bedrock API key in `x-api-key`,
//! against the fake Messages server on 127.0.0.1.

#[cfg(test)]
mod over_http {
    use std::sync::Arc;

    use glave_aws::{Endpoint, Region, SigV4, Source};
    use glave_event::Part;
    use glave_ports::auth::{Auth, KeyHeader};
    use glave_ports::block_on;
    use glave_ports::testing::{FAKE_SECRET, ManualClock};
    use glave_ports::{Message, ModelProvider, ModelRequest, ToolChoice};
    use glave_provider_anthropic::fake::{FakeServer, Reply};
    use glave_provider_anthropic::{Anthropic, Config};
    use jiff::Timestamp;

    fn ask(base_url: &str, auth: Auth) -> String {
        let provider = Anthropic::new(Config {
            base_url: base_url.to_owned(),
            auth,
            provider: "bedrock".into(),
            ..Config::new(None)
        })
        .unwrap();
        let request = ModelRequest {
            model: "anthropic.claude-opus-5-5".into(),
            system: String::new(),
            messages: vec![Message::User {
                parts: vec![Part::Text { text: "hi".into() }],
            }],
            tools: Vec::new(),
            tool_choice: ToolChoice::Auto,
            max_output_tokens: None,
            purpose: glave_ports::Purpose::Main,
            threads: Vec::new(),
            effort: None,
        };
        let completion = block_on!(provider.complete(&request, &mut |_| {})).unwrap();
        match completion.parts.as_slice() {
            [Part::Text { text }] => text.clone(),
            other => panic!("expected one text part, got {other:?}"),
        }
    }

    #[test]
    fn each_endpoint_has_its_anthropic_route() {
        let region = Region::new("us-east-1").unwrap();
        assert_eq!(
            Endpoint::Runtime.anthropic_base_url(&region),
            "https://bedrock-runtime.us-east-1.amazonaws.com/anthropic"
        );
        assert_eq!(
            Endpoint::Mantle.anthropic_base_url(&region),
            "https://bedrock-mantle.us-east-1.api.aws/anthropic"
        );
    }

    #[test]
    fn claude_is_asked_at_the_anthropic_route_signed_with_sigv4() {
        let server = FakeServer::start(vec![Reply::text("hello from bedrock")]).unwrap();
        let region = Region::new("us-east-1").unwrap();
        let env = |name: &str| match name {
            "AWS_ACCESS_KEY_ID" => Some("AKIDEXAMPLE".to_owned()),
            "AWS_SECRET_ACCESS_KEY" => Some("wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".to_owned()),
            _ => None,
        };
        let source = Source::from_env(&env, &region).unwrap().unwrap();
        let at = "2026-10-12T18:00:00Z".parse::<Timestamp>().unwrap();
        let clock = ManualClock::at_second(at.as_second());
        let signer = SigV4::new(source, region, Endpoint::Runtime, Arc::new(clock)).unwrap();
        let base = format!("{}/anthropic", server.url());
        assert_eq!(
            ask(&base, Auth::Changing(Arc::new(signer))),
            "hello from bedrock"
        );
        let received = server.received();
        assert_eq!(received[0].path, "/anthropic/v1/messages");
        assert_eq!(received[0].version.as_deref(), Some("2023-06-01"));
        assert_eq!(received[0].body["model"], "anthropic.claude-opus-5-5");
        assert_eq!(received[0].body["stream"], true);
        assert_eq!(received[0].api_key, None);
        let authorization = received[0].authorization.as_deref().unwrap();
        assert!(
            authorization.starts_with(
                "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20261012/us-east-1/bedrock/aws4_request, \
                 SignedHeaders=anthropic-version;content-type;host;x-amz-date, Signature="
            ),
            "{authorization}"
        );
    }

    #[test]
    fn a_bedrock_api_key_goes_in_x_api_key() {
        let server = FakeServer::start(vec![Reply::text("hi")]).unwrap();
        let base = format!("{}/anthropic", server.url());
        ask(&base, Auth::key(KeyHeader::XApiKey, FAKE_SECRET));
        let received = server.received();
        assert_eq!(received[0].path, "/anthropic/v1/messages");
        assert_eq!(received[0].api_key.as_deref(), Some(FAKE_SECRET));
        assert_eq!(received[0].authorization, None);
    }
}
