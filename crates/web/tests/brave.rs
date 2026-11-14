//! Brave's Search API as glave's web search reaches it, against a stand-in on this computer
//! (glave-ports' fake HTTP server), never Brave itself.

use glave_event::{Nanos, Secret};
use glave_ports::testing::{FakeHttp, any_secret};
use glave_ports::{
    Failure, Hit, Message, ModelProvider as _, ModelRequest, ProviderError, Purpose, SearchAsk,
    ToolChoice, block_on,
};
use glave_web::Brave;
use serde_json::{Value, json};

fn request() -> ModelRequest {
    ModelRequest {
        model: String::new(),
        system: String::new(),
        messages: Vec::<Message>::new(),
        tools: Vec::new(),
        tool_choice: ToolChoice::None,
        max_output_tokens: None,
        purpose: Purpose::WebSearch,
        effort: None,
        files: Vec::new(),
        payer: None,
        route: None,
    }
}

fn ask(query: &str, count: u8) -> SearchAsk {
    SearchAsk {
        query: query.to_owned(),
        count,
    }
}

/// Brave's answer with these results, as its API gives them.
fn answer(results: &Value) -> String {
    json!({"type": "search", "query": {"original": "q"}, "web": {"type": "search", "results": results}})
        .to_string()
}

#[test]
fn a_search_sends_the_query_and_key_to_brave_and_reads_its_web_results() {
    let results = json!([
        {"title": "<strong>Tokio</strong> - An asynchronous Rust runtime", "url": "https://tokio.rs/",
         "description": "Tokio is an <strong>asynchronous</strong> runtime for Rust &amp; more.", "age": "1 day"},
        {"title": "No address"},
        {"title": "smol", "url": "https://github.com/smol-rs/smol"}
    ]);
    let stand_in = FakeHttp::start(vec![(200, answer(&results))]).unwrap();
    let key = any_secret();
    let url = format!("{}/res/v1/web/search", stand_in.url);
    let brave = Brave::new(Secret::new(key.clone()), Some(&url)).unwrap();
    assert!(!format!("{brave:?}").contains(&key), "never in Debug");
    let found = block_on!(brave.search(&request(), &ask("rust async runtimes", 50))).unwrap();
    assert_eq!(
        found.hits,
        [
            Hit {
                title: "Tokio - An asynchronous Rust runtime".into(),
                url: "https://tokio.rs/".into(),
                snippet: "Tokio is an asynchronous runtime for Rust & more.".into(),
            },
            Hit {
                title: "smol".into(),
                url: "https://github.com/smol-rs/smol".into(),
                snippet: String::new(),
            },
        ]
    );
    let sent = stand_in.received();
    let [sent] = sent.as_slice() else {
        panic!("one request: {sent:?}")
    };
    assert_eq!(sent.method, "GET");
    assert_eq!(
        sent.path, "/res/v1/web/search?q=rust+async+runtimes&count=20",
        "at most Brave's 20"
    );
    assert_eq!(sent.headers.get("x-subscription-token"), Some(&key));
    assert_eq!(
        sent.headers.get("accept").map(String::as_str),
        Some("application/json")
    );
}

#[test]
fn a_refusal_says_brave_s_status_and_never_the_key() {
    let stand_in =
        FakeHttp::start(vec![(429, json!({"error": "rate limited"}).to_string())]).unwrap();
    let key = any_secret();
    let brave = Brave::new(Secret::new(key.clone()), Some(&stand_in.url)).unwrap();
    let refused = block_on!(brave.search(&request(), &ask("q", 5))).unwrap_err();
    assert!(
        matches!(&refused, ProviderError::Http { status: 429, .. }),
        "{refused:?}"
    );
    assert!(!refused.unanswered(), "Brave bills no refused search");
    assert!(!refused.to_string().contains(&key));
}

#[test]
fn a_search_that_never_reached_brave_is_said_so_and_was_not_billed() {
    // The discard port: nothing listens there.
    let brave = Brave::new(Secret::new(any_secret()), Some("http://127.0.0.1:9/")).unwrap();
    let missed = block_on!(brave.search(&request(), &ask("q", 5))).unwrap_err();
    assert!(
        matches!(
            &missed,
            ProviderError::Unserved {
                failure: Failure::Unreachable,
                ..
            }
        ),
        "{missed:?}"
    );
    assert!(
        !missed.unanswered(),
        "nothing was sent, so nothing is charged"
    );
}

#[test]
fn brave_serves_searches_alone() {
    let brave = Brave::new(Secret::new(any_secret()), None).unwrap();
    assert!(brave.serves(Purpose::WebSearch));
    assert!(!brave.serves(Purpose::Main) && !brave.serves(Purpose::ImageGenerate));
    assert!(Brave::new(Secret::new(any_secret()), Some("ftp://example.com/")).is_err());
    assert_eq!(Brave::price().nanos, Nanos::new(5_000_000), "$5 for 1,000");
}
