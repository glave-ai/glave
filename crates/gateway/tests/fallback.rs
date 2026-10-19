//! Fallback: a target that cannot honour a request is skipped, never sent a degraded one; a
//! failure that may pass moves to the next target and any other is final; and a conversation
//! holding opaque reasoning stays with the model that produced it, or is stripped, on record.

use glave_event::{BlobRef, Part, Sha256};
use glave_gateway::{Capabilities, Gateway, Routes, Target};
use glave_ports::{Failure, Message, ModelProvider, ModelRequest, ProviderError, Purpose};

use crate::support::{Fake, block_on, request, with_reasoning, with_tool};

/// The name of the target that served `request`, or why none did.
fn serve(targets: Vec<Target>, request: &ModelRequest) -> Result<String, ProviderError> {
    let routes = Routes::new(targets).map_err(|e| ProviderError::Protocol(e.to_string()))?;
    let answer = block_on!(Gateway::new(routes).complete(request, &mut |_| {}))?;
    Ok(answer
        .served
        .map(|served| served.target)
        .unwrap_or_default())
}

fn limited(target: Target, change: impl FnOnce(&mut Capabilities)) -> Target {
    let mut can = Capabilities::default();
    change(&mut can);
    target.with_capabilities(can)
}

#[test]
fn a_target_without_tools_is_skipped_when_tools_are_offered() {
    let fake = Fake::new("p");
    let targets = || {
        vec![
            limited(fake.target("no-tools"), |can| can.tools = false),
            fake.target("tools"),
        ]
    };
    assert_eq!(
        serve(targets(), &with_tool(request(Purpose::Main))).unwrap(),
        "p/tools"
    );
    assert_eq!(
        serve(targets(), &request(Purpose::Main)).unwrap(),
        "p/no-tools"
    );
    let models: Vec<String> = fake.seen().into_iter().map(|r| r.model).collect();
    assert_eq!(
        models,
        ["tools", "no-tools"],
        "nothing was sent to a skipped target"
    );
}

#[test]
fn a_target_without_images_is_skipped_when_the_conversation_has_one() {
    let fake = Fake::new("p");
    let mut asked = request(Purpose::Main);
    asked.messages.push(Message::User {
        parts: vec![Part::File {
            blob: BlobRef {
                sha256: Sha256::new("a".repeat(64)).unwrap(),
                bytes: 10,
                media_type: "image/png".into(),
            },
            name: None,
        }],
    });
    let targets = vec![
        limited(fake.target("blind"), |can| can.images = false),
        fake.target("sighted"),
    ];
    assert_eq!(serve(targets, &asked).unwrap(), "p/sighted");
}

#[test]
fn a_target_with_too_small_a_window_is_skipped() {
    let fake = Fake::new("p");
    let mut asked = request(Purpose::Main);
    asked.system = "x".repeat(4_000); // about 1,000 tokens, plus 100 of answer
    let small = || limited(fake.target("small"), |can| can.context_tokens = Some(1_000));
    let large = limited(fake.target("large"), |can| can.context_tokens = Some(2_000));
    assert_eq!(serve(vec![small(), large], &asked).unwrap(), "p/large");
    let refused = serve(vec![small()], &asked).unwrap_err();
    assert_eq!(
        refused.failure(),
        Failure::TooLong,
        "so the harness compacts"
    );
    assert!(
        refused
            .to_string()
            .contains("p/small's context window of 1000"),
        "{refused}"
    );
    let tools = limited(fake.target("no-tools"), |can| can.tools = false);
    let mixed = serve(vec![small(), tools], &with_tool(asked)).unwrap_err();
    assert_eq!(mixed.failure(), Failure::Rejected);
}

#[test]
fn a_failure_that_may_pass_moves_to_the_next_target() {
    for error in [
        ProviderError::Http {
            status: 429,
            body: "slow down".into(),
        },
        ProviderError::Http {
            status: 503,
            body: "overloaded".into(),
        },
        ProviderError::Timeout { seconds: 300 },
        ProviderError::Transport("reset".into()),
    ] {
        let first = Fake::failing("first", error.clone());
        let second = Fake::new("second");
        let targets = vec![first.target("m"), second.target("m")];
        assert_eq!(
            serve(targets, &request(Purpose::Main)).unwrap(),
            "second/m",
            "{error}"
        );
    }
}

#[test]
fn any_other_failure_is_final() {
    for error in [
        ProviderError::Http {
            status: 401,
            body: "bad key".into(),
        },
        ProviderError::Http {
            status: 400,
            body: "maximum context length is 8192 tokens".into(),
        },
        ProviderError::Protocol("garbled".into()),
    ] {
        let first = Fake::failing("first", error.clone());
        let second = Fake::new("second");
        let targets = vec![first.target("m"), second.target("m")];
        assert_eq!(serve(targets, &request(Purpose::Main)), Err(error));
        assert_eq!(second.seen(), []);
    }
}

#[test]
fn when_every_target_fails_the_last_failure_is_returned() {
    let first = Fake::failing("first", ProviderError::Timeout { seconds: 1 });
    let second = Fake::failing("second", ProviderError::Transport("down".into()));
    let targets = vec![first.target("m"), second.target("m")];
    assert_eq!(
        serve(targets, &request(Purpose::Main)),
        Err(ProviderError::Transport("down".into())),
        "transient, so the harness's own retries try again"
    );
}

#[test]
fn opaque_reasoning_stays_with_the_model_that_produced_it() {
    let other = Fake::new("openai");
    let own = Fake::new("anthropic");
    let asked = with_reasoning(request(Purpose::Main), "anthropic", "sonnet");
    let targets = vec![other.target("gpt"), own.target("sonnet")];
    assert_eq!(serve(targets, &asked).unwrap(), "anthropic/sonnet");
    assert_eq!(
        other.seen(),
        [],
        "the first target cannot read the reasoning"
    );
    assert_eq!(own.seen()[0].messages, asked.messages, "sent byte for byte");
}

#[test]
fn sticky_reasoning_falls_back_only_to_targets_that_accept_it() {
    let own = Fake::failing("anthropic", ProviderError::Timeout { seconds: 1 });
    let other = Fake::new("openai");
    let asked = with_reasoning(request(Purpose::Main), "anthropic", "sonnet");
    let targets = vec![own.target("sonnet"), other.target("gpt")];
    assert_eq!(
        serve(targets, &asked),
        Err(ProviderError::Timeout { seconds: 1 })
    );
    assert_eq!(other.seen(), []);
}

#[test]
fn reasoning_no_target_accepts_is_stripped_and_reported() {
    let other = Fake::new("openai");
    let asked = with_reasoning(request(Purpose::Main), "anthropic", "sonnet");
    let gateway = Gateway::new(Routes::new(vec![other.target("gpt")]).unwrap());
    let answer = block_on!(gateway.complete(&asked, &mut |_| {})).unwrap();
    assert!(answer.served.unwrap().reasoning_stripped);
    let sent = &other.seen()[0].messages;
    let Message::Assistant { parts, .. } = &sent[1] else {
        panic!("{sent:?}")
    };
    assert_eq!(
        parts,
        &[Part::Text { text: "Hi.".into() }],
        "the text stays"
    );
    let plain = block_on!(gateway.complete(&request(Purpose::Main), &mut |_| {})).unwrap();
    assert!(!plain.served.unwrap().reasoning_stripped);
}

#[test]
fn a_target_that_takes_no_reasoning_back_is_not_sticky() {
    let own = Fake::new("anthropic");
    let asked = with_reasoning(request(Purpose::Main), "anthropic", "sonnet");
    let target = limited(own.target("sonnet"), |can| can.reasoning = false);
    let gateway = Gateway::new(Routes::new(vec![target]).unwrap());
    let answer = block_on!(gateway.complete(&asked, &mut |_| {})).unwrap();
    assert!(answer.served.unwrap().reasoning_stripped);
}
