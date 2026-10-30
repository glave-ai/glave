//! Credits, the real binary through the person's server: with `[credits]` in its settings and
//! nothing added, a turn is refused with a sentence before the model is asked; without, a turn
//! goes as it always did.

use glave_provider_openai::fake::{FakeServer, Reply};

use crate::http::serve_http;
use crate::support::*;

#[test]
fn with_nothing_added_a_turn_is_refused_before_the_model_is_asked() {
    let place = place("credits");
    std::fs::create_dir_all(&place.home).unwrap();
    let settings = "[credits]\nmonthly_limit_usd = 10\n";
    std::fs::write(place.home.join("config.toml"), settings).unwrap();
    let model = FakeServer::start(vec![Reply::text("Hi.")]).unwrap();
    let (_served, _) = serve_http(&place, &model, &["--model", "m"]);
    let turn = glave(&place, &model, &["--model", "m", "hello"], "");
    let said = text(&turn);
    assert!(!turn.status.success(), "{said}");
    assert!(said.contains("you're out of credits"), "{said}");
    assert!(model.received().is_empty(), "the model was never asked");
}

#[test]
fn a_glave_that_keeps_no_credits_charges_nothing() {
    let place = place("no-credits");
    let model = FakeServer::start(vec![Reply::text("Hi.")]).unwrap();
    let (_served, _) = serve_http(&place, &model, &["--model", "m"]);
    let turn = glave(&place, &model, &["--model", "m", "hello"], "");
    assert!(turn.status.success(), "{}", text(&turn));
}
