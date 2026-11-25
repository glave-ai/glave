//! What a call is charged when its provider says too little of what it used
//! (`specs/gateway.md`, "Credits"): an answer with no usage is charged what it read and what
//! came, never nothing; a reported cost below nothing is not believed; a call sent and left
//! unanswered is charged what it read, one that never reached its provider nothing; a tool
//! call's arguments count as what came; images and PDFs sent inline are held at what they may
//! read; and a paid turn is always sent the cap its hold counts.

use std::sync::Arc;

use glave_event::Cost;
use glave_gateway::{Credits, Gateway, Routes, Target};
use glave_ports::testing::FaultyLog;
use glave_ports::{
    InlineFile, Message, ModelProvider, ModelRequest, Payer, ProviderError, Purpose, Unpaid, digest,
};

use crate::support::{
    Fake, answer, balance, block_on, dollars, funded, org, person, price, request,
};

/// What the test request reads at a billionth of a dollar a token, "Be brief." and "hello",
/// 14 bytes at 4 a token, and what "Twelve bytes" of answer comes to at 4 bytes a token.
const READ: i64 = 4;
const CAME: i64 = 3;

fn over(target: Target, credits: &Arc<Credits<FaultyLog>>) -> Gateway {
    Gateway::new(Routes::new(vec![target]).unwrap()).with_credits(Arc::clone(credits))
}

fn paid(mut asked: ModelRequest) -> ModelRequest {
    asked.payer = Some(Payer {
        org: org(),
        person: Some(person()),
    });
    asked
}

fn ask(gateway: &Gateway) -> Result<glave_ports::Completion, ProviderError> {
    block_on!(gateway.complete(&paid(request(Purpose::Main)), &mut |_| {}))
}

/// A fake at a billionth of a dollar a token that says "Twelve bytes", then answers `next`.
fn answering(next: Result<glave_ports::Completion, ProviderError>) -> Arc<Fake> {
    let mut fake = Fake::new("p");
    Arc::get_mut(&mut fake).unwrap().says = Some("Twelve bytes".into());
    fake.script.lock().unwrap().push_back(next);
    fake
}

#[test]
fn an_answer_that_says_nothing_of_what_it_used_is_charged_what_it_read_and_what_came() {
    let credits = funded(1_000);
    let fake = answering(Ok(answer("p", "m", 0, 0)));
    let completion = ask(&over(
        fake.target("m").with_price(price(1_000_000)),
        &credits,
    ))
    .unwrap();
    assert_eq!(balance(&credits), 1_000 - READ - CAME, "never nothing");
    assert_eq!(completion.cost, None, "its receipt says no cost, not $0.00");
}

#[test]
fn a_reported_cost_below_nothing_is_not_believed_and_the_call_is_charged_at_its_price() {
    let credits = funded(1_000);
    let mut refund = answer("p", "m", 100, 10);
    refund.cost = Some(Cost {
        amount: dollars(-500),
        price_list: "p:reported".into(),
    });
    let fake = answering(Ok(refund));
    ask(&over(
        fake.target("m").with_price(price(1_000_000)),
        &credits,
    ))
    .unwrap();
    assert_eq!(balance(&credits), 1_000 - 110, "110 tokens at their price");
}

#[test]
fn a_call_sent_and_left_unanswered_is_charged_what_it_read_and_one_never_sent_nothing() {
    let credits = funded(1_000);
    let silent = Fake::failing("p", ProviderError::Timeout { seconds: 300 });
    assert!(
        ask(&over(
            silent.target("m").with_price(price(1_000_000)),
            &credits
        ))
        .is_err()
    );
    assert_eq!(
        balance(&credits),
        1_000 - READ,
        "its provider billed what it read"
    );
    let unreached = Fake::failing("p", ProviderError::Unreached("refused".into()));
    assert!(
        ask(&over(
            unreached.target("m").with_price(price(1_000_000)),
            &credits
        ))
        .is_err()
    );
    assert_eq!(balance(&credits), 1_000 - READ, "nothing was sent");
}

#[test]
fn a_tool_call_s_arguments_count_as_what_came() {
    let credits = funded(1_000);
    let mut fake = Fake::new("p");
    Arc::get_mut(&mut fake).unwrap().calls_with = Some(r#"{"path":"a"}"#.into());
    let broken = ProviderError::AfterOutput(Box::new(ProviderError::Protocol("cut".into())));
    fake.script.lock().unwrap().push_back(Err(broken));
    assert!(
        ask(&over(
            fake.target("m").with_price(price(1_000_000)),
            &credits
        ))
        .is_err()
    );
    assert_eq!(
        balance(&credits),
        1_000 - READ - CAME,
        "12 bytes of arguments"
    );
}

/// What the test request holds with `file` sent inline beside it, at `target`, refused for
/// want of credits: the hold it asked for.
fn held_with(file: InlineFile, target: Target) -> i64 {
    let credits = funded(1);
    let mut asked = request(Purpose::Main);
    let blob = glave_event::BlobRef {
        sha256: file.sha256.clone(),
        bytes: 1,
        media_type: file.media_type.clone(),
    };
    asked.messages.push(Message::User {
        parts: vec![glave_event::Part::File {
            blob,
            name: None,
            resized: None,
        }],
    });
    asked.files = vec![file];
    let gateway = over(target, &credits);
    match block_on!(gateway.complete(&paid(asked), &mut |_| {})) {
        Err(ProviderError::Unpaid(Unpaid::Short { needs, .. })) => needs.nanos.get(),
        other => panic!("expected the call refused for want of credits, got {other:?}"),
    }
}

#[test]
fn an_image_or_a_pdf_sent_inline_is_held_at_what_it_may_read_within_the_window() {
    let image = || InlineFile {
        sha256: digest(b"png"),
        media_type: "image/png".into(),
        data: "cG5n".into(),
    };
    let fake = Fake::new("p");
    let target = || fake.target("m").with_price(price(1_000_000));
    // Its 5 tokens of text, read cautiously, its 100 of answer, and the most an image reads.
    assert_eq!(held_with(image(), target()), 5 + 8_192 + 100);
    // 4,000 characters of base64 are 3,000 bytes: 1,000 tokens, read as cautiously as text.
    let pdf = InlineFile {
        sha256: digest(b"pdf"),
        media_type: "application/pdf".into(),
        data: "A".repeat(4_000),
    };
    assert_eq!(held_with(pdf, target()), 5 + 1_000 + 100);
    let mut small = target();
    small.can.context_tokens = Some(2_000);
    assert_eq!(
        held_with(image(), small),
        2_000 + 100,
        "never more read than the window holds"
    );
}

#[test]
fn a_paid_turn_on_a_model_nothing_is_known_of_is_sent_the_cap_its_hold_counts() {
    let credits = funded(1_000_000);
    let fake = Fake::new("p");
    let gateway = over(fake.target("m").with_price(price(1_000_000)), &credits);
    let mut turn = request(Purpose::Main);
    turn.max_output_tokens = Some(32_000);
    block_on!(gateway.complete(&turn, &mut |_| {})).unwrap();
    block_on!(gateway.complete(&paid(turn), &mut |_| {})).unwrap();
    let sent: Vec<Option<u32>> = (fake.seen().into_iter())
        .map(|sent| sent.max_output_tokens)
        .collect();
    assert_eq!(
        sent,
        [None, Some(32_000)],
        "unpaid, not sent, as it may not fit; paid, the answer is bounded by what is held"
    );
}
