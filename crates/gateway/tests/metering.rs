//! What a call is charged when its provider says too little of what it used
//! (`specs/gateway.md`, "Credits"): an answer with no usage, or usage estimated from a stream
//! that ended early, is charged at least what it read and what came; a call sent and left
//! unanswered is charged what it read, one refused or never sent nothing; a tool call's
//! arguments count as what came; and images and PDFs sent inline are held and charged at what
//! they may read, a PDF never past a few pages' worth nor anything past the window.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use glave_event::{TokenCounts, Usage};
use glave_gateway::{Credits, Gateway, Routes, Target};
use glave_ports::testing::FaultyLog;
use glave_ports::{
    Failure, InlineFile, Message, ModelProvider, ModelRequest, Payer, ProviderError, Purpose,
    Unpaid, digest,
};
use tokio::sync::Semaphore;

use crate::support::{Fake, answer, balance, block_on, funded, org, person, price, request};

/// What the test request reads at a billionth of a dollar a token, "Be brief." and "hello",
/// 14 bytes at 4 a token, and what "Twelve bytes" of answer comes to at 4 bytes a token.
const READ: i64 = 4;
const CAME: i64 = 3;

/// What a PDF is counted at, at most: twenty pages of 1,500 tokens.
const MOST_PDF: i64 = 30_000;

fn over(target: Target, credits: &Arc<Credits<FaultyLog>>) -> Gateway {
    Gateway::new(Routes::new(vec![target]).unwrap()).with_credits(Arc::clone(credits))
}

fn priced(fake: &Arc<Fake>) -> Target {
    fake.target("m").with_price(price(1_000_000))
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

/// A fake that says "Twelve bytes" as its answer begins.
fn saying() -> Arc<Fake> {
    let mut fake = Fake::new("p");
    Arc::get_mut(&mut fake).unwrap().says = Some("Twelve bytes".into());
    fake
}

/// A fake that says "Twelve bytes", then answers `next`.
fn answering(next: Result<glave_ports::Completion, ProviderError>) -> Arc<Fake> {
    let fake = saying();
    fake.script.lock().unwrap().push_back(next);
    fake
}

#[test]
fn an_answer_that_says_nothing_of_what_it_used_is_charged_what_it_read_and_what_came() {
    let credits = funded(1_000);
    let fake = answering(Ok(answer("p", "m", 0, 0)));
    let completion = ask(&over(priced(&fake), &credits)).unwrap();
    assert_eq!(balance(&credits), 1_000 - READ - CAME, "never nothing");
    assert_eq!(completion.cost, None, "its receipt says no cost, not $0.00");
}

#[test]
fn usage_estimated_from_a_stream_that_ended_early_is_charged_no_less_than_what_came() {
    let credits = funded(1_000);
    let mut early = answer("p", "m", 0, 0);
    let counts = TokenCounts {
        input: 1,
        output: 1,
        ..TokenCounts::default()
    };
    early.usage = Usage::new(counts, true, None).unwrap();
    ask(&over(priced(&answering(Ok(early))), &credits)).unwrap();
    assert_eq!(
        balance(&credits),
        1_000 - READ - CAME,
        "not the 2 it counted"
    );
}

#[test]
fn a_call_sent_and_left_unanswered_is_charged_what_it_read_and_one_refused_or_unsent_nothing() {
    let credits = funded(1_000);
    let silent = Fake::failing("p", ProviderError::Timeout { seconds: 300 });
    assert!(ask(&over(priced(&silent), &credits)).is_err());
    assert_eq!(balance(&credits), 1_000 - READ, "billed what it read");
    for nothing in [
        ProviderError::Unserved {
            failure: Failure::Unreachable,
            reason: "connection refused".into(),
        },
        ProviderError::NoOutput("finish reason error".into()),
    ] {
        let fake = Fake::failing("p", nothing.clone());
        assert!(ask(&over(priced(&fake), &credits)).is_err());
        assert_eq!(balance(&credits), 1_000 - READ, "{nothing}");
    }
}

#[test]
fn a_tool_call_s_arguments_count_as_what_came() {
    let credits = funded(1_000);
    let mut fake = Fake::new("p");
    Arc::get_mut(&mut fake).unwrap().calls_with = Some(r#"{"path":"a"}"#.into());
    let broken = ProviderError::AfterOutput(Box::new(ProviderError::Protocol("cut".into())));
    fake.script.lock().unwrap().push_back(Err(broken));
    assert!(ask(&over(priced(&fake), &credits)).is_err());
    assert_eq!(
        balance(&credits),
        1_000 - READ - CAME,
        "12 bytes of arguments"
    );
}

/// The test request with `file` sent inline beside it.
fn with_file(file: InlineFile) -> ModelRequest {
    let mut asked = paid(request(Purpose::Main));
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
    asked
}

fn image() -> InlineFile {
    InlineFile {
        sha256: digest(b"png"),
        media_type: "image/png".into(),
        data: "cG5n".into(),
    }
}

/// A PDF of `characters` of base64: three bytes in every four.
fn pdf(characters: usize) -> InlineFile {
    InlineFile {
        sha256: digest(b"pdf"),
        media_type: "application/pdf".into(),
        data: "A".repeat(characters),
    }
}

/// What the test request holds with `file` sent inline beside it, at `target`, refused for
/// want of credits: the hold it asked for.
fn held_with(file: InlineFile, target: Target) -> i64 {
    let gateway = over(target, &funded(1));
    match block_on!(gateway.complete(&with_file(file), &mut |_| {})) {
        Err(ProviderError::Unpaid(Unpaid::Short { needs, .. })) => needs.nanos.get(),
        other => panic!("expected the call refused for want of credits, got {other:?}"),
    }
}

#[test]
fn an_image_or_a_pdf_sent_inline_is_held_at_what_it_may_read_within_the_window() {
    let fake = Fake::new("p");
    // Its 5 tokens of text, read cautiously, its 100 of answer, and what a chat model reads
    // of an image.
    assert_eq!(held_with(image(), priced(&fake)), 5 + 1_600 + 100);
    // 4,000 characters of base64 are 3,000 bytes: 1,000 tokens, read as cautiously as text.
    assert_eq!(held_with(pdf(4_000), priced(&fake)), 5 + 1_000 + 100);
    // A scanned PDF of 15 MB is a few pages, never held at what its bytes would say.
    let scanned = || pdf(20_000_000);
    assert_eq!(held_with(scanned(), priced(&fake)), 5 + MOST_PDF + 100);
    let mut small = priced(&fake);
    small.can.context_tokens = Some(2_000);
    assert_eq!(
        held_with(scanned(), small),
        2_000 + 100,
        "never more read than the window holds"
    );
}

/// What cancelling the test request, with `file` sent inline, after "Twelve bytes" came,
/// charges at `fake`'s `target`.
fn cancelled_with(file: InlineFile, fake: &Arc<Fake>, target: Target) -> i64 {
    let credits = funded(1_000_000);
    let gateway = Arc::new(over(target, &credits));
    block_on!(async {
        let asked = with_file(file);
        let calling = Arc::clone(&gateway);
        let call = tokio::spawn(async move { calling.complete(&asked, &mut |_| {}).await });
        // Sent, and waiting at the fake's gate for an answer, where it is cancelled.
        for _ in 0..1_000 {
            if fake.calls.load(Ordering::SeqCst) > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(fake.calls.load(Ordering::SeqCst), 1, "sent");
        call.abort();
        assert!(call.await.unwrap_err().is_cancelled());
    });
    1_000_000 - balance(&credits)
}

#[test]
fn a_call_cancelled_with_a_large_pdf_is_charged_a_few_pages_of_it_not_its_bytes() {
    let gated = || {
        let mut fake = Fake::gated("p", Arc::new(Semaphore::new(0)));
        Arc::get_mut(&mut fake).unwrap().says = Some("Twelve bytes".into());
        fake
    };
    let fake = gated();
    let charged = cancelled_with(pdf(20_000_000), &fake, priced(&fake));
    assert_eq!(charged, READ + MOST_PDF + CAME);
    let fake = gated();
    let mut small = priced(&fake);
    small.can.context_tokens = Some(2_000);
    let charged = cancelled_with(pdf(20_000_000), &fake, small);
    assert_eq!(
        charged,
        2_000 + CAME,
        "never more read than the window holds"
    );
}
