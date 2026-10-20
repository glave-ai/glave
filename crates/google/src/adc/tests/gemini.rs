//! Gemini through Vertex's OpenAI-compatible endpoint, end to end: the OpenAI-compatible
//! provider, with the metadata server's token as its bearer.

use glave_event::Part;
use glave_ports::{Message, ModelProvider, ModelRequest, ToolChoice};
use glave_provider_openai::fake::{FakeServer, Reply};
use glave_provider_openai::{Config, IDLE_TIMEOUT, OpenAiCompatible};

use super::*;

#[test]
fn gemini_is_asked_with_the_workload_s_token() {
    let google = FakeHttp::start(vec![token("ya29.gke", 3599)]).unwrap();
    let vertex = FakeServer::start(vec![Reply::text("hello from gemini")]).unwrap();
    let pod = Pod::new("gemini");
    let home = pod.folder.display().to_string();
    let auth = pod.adc(&[("GCE_METADATA_HOST", &google.host), ("HOME", &home)]);
    let provider = OpenAiCompatible::new(Config {
        base_url: vertex.url().to_owned(),
        auth,
        provider: "vertex".into(),
        idle_timeout: IDLE_TIMEOUT,
    })
    .unwrap();
    let request = ModelRequest {
        model: "google/gemini-3.5-flash".into(),
        system: String::new(),
        messages: vec![Message::User {
            parts: vec![Part::Text { text: "hi".into() }],
        }],
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        max_output_tokens: None,
        purpose: glave_ports::Purpose::Main,
        threads: Vec::new(),
        files: Vec::new(),
        effort: None,
    };
    let completion = block_on!(provider.complete(&request, &mut |_| {})).unwrap();
    assert_eq!(
        completion.parts,
        vec![Part::Text {
            text: "hello from gemini".into()
        }]
    );
    let received = vertex.received();
    assert_eq!(
        received[0].authorization.as_deref(),
        Some("Bearer ya29.gke")
    );
    assert_eq!(received[0].body["model"], "google/gemini-3.5-flash");
    assert_eq!(google.received().len(), 1);
}
