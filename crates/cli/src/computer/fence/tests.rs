use std::net::{Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::time::{Instant, sleep};

use super::{HOLD_WITHIN, Held, KNOCK, Knocked, address, held_by, knock};

/// A runtime whose clock moves only when nothing else can run, so the knocks' waits and the
/// fence's limit take no real time.
fn paused() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap()
}

const SILENCE: Knocked = Knocked::Silence;

/// A knock answered in `ms` milliseconds.
const fn answer(ms: u64) -> Knocked {
    Knocked::Answer {
        took: Duration::from_millis(ms),
        refused: false,
    }
}

/// A knock refused, as a plugin that rejects rather than drops answers each.
const REFUSAL: Knocked = Knocked::Answer {
    took: Duration::from_millis(1),
    refused: true,
};

/// A knock that comes to `knocked`, taking as long as it would.
async fn knocked(knocked: Knocked) -> Knocked {
    match knocked {
        Knocked::Silence => sleep(KNOCK).await,
        Knocked::Answer { took, .. } => sleep(took).await,
    }
    knocked
}

type Knocks = Box<dyn FnMut() -> Pin<Box<dyn Future<Output = Knocked>>>>;

/// Knocks that come to `script` in turn, then to its last again and again.
fn scripted(script: &'static [Knocked]) -> Knocks {
    let last = *script.last().unwrap();
    let mut script = script.iter().copied();
    Box::new(move || Box::pin(knocked(script.next().unwrap_or(last))))
}

/// Knocks that come to `script` in turn, over and over.
fn cycled(script: &'static [Knocked]) -> Knocks {
    let mut script = script.iter().copied().cycle();
    Box::new(move || Box::pin(knocked(script.next().unwrap())))
}

fn node() -> SocketAddr {
    SocketAddr::from(([10, 0, 0, 7], 10250))
}

/// What `knocks` came to on a paused clock, and how long it took there.
fn fenced(knocks: Knocks) -> (Result<Held, String>, Duration) {
    paused().block_on(async {
        let started = Instant::now();
        let held = held_by(knocks, node(), HOLD_WITHIN).await;
        (held.map_err(|why| why.to_string()), started.elapsed())
    })
}

#[test]
fn three_unanswered_knocks_in_a_row_hold_the_fence_saying_the_slowest_answer() {
    static SCRIPT: [Knocked; 5] = [answer(1), answer(5), SILENCE, SILENCE, SILENCE];
    let (held, took) = fenced(scripted(&SCRIPT));
    let slowest = Some(Duration::from_millis(5));
    assert_eq!(held, Ok(Held { knocks: 5, slowest }));
    assert!(took < Duration::from_secs(1), "{took:?}");
}

#[test]
fn a_policy_that_holds_from_the_start_takes_three_knocks() {
    let (held, took) = fenced(scripted(&[SILENCE]));
    assert_eq!(
        held,
        Ok(Held {
            knocks: 3,
            slowest: None
        })
    );
    assert_eq!(took, KNOCK * 3, "the silence floor");
}

#[test]
fn an_answer_among_the_silences_starts_the_count_again() {
    static SCRIPT: [Knocked; 6] = [SILENCE, SILENCE, answer(1), SILENCE, SILENCE, SILENCE];
    let (held, _) = fenced(scripted(&SCRIPT));
    assert_eq!(held.map(|held| held.knocks), Ok(6));
}

#[test]
fn a_node_that_answers_now_and_then_never_holds_it_and_says_where_and_what_next() {
    // A lost or slow answer reads as one silence; never as three in a row while it answers.
    static SCRIPT: [Knocked; 3] = [SILENCE, SILENCE, answer(1)];
    let (held, took) = fenced(cycled(&SCRIPT));
    let said = held.unwrap_err();
    assert!(said.contains("10.0.0.7:10250"), "{said}");
    assert!(
        said.contains("isn't enforcing its network policy"),
        "{said}"
    );
    assert!(said.contains("next wake tries again"), "{said}");
    assert!(
        took >= HOLD_WITHIN && took < HOLD_WITHIN + Duration::from_secs(1),
        "it gives up at its limit: {took:?}"
    );
}

#[test]
fn a_plugin_that_refuses_every_knock_never_holds_it_and_is_said_to_refuse() {
    let (held, _) = fenced(scripted(&[REFUSAL]));
    let said = held.unwrap_err();
    assert!(said.contains("was refused"), "{said}");
    assert!(said.contains("rejects what a policy refuses"), "{said}");
    assert!(!said.contains("isn't enforcing"), "{said}");
}

#[test]
fn a_knock_at_a_listener_or_a_closed_port_is_answered() {
    glave_ports::block_on!(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let at = listener.local_addr().unwrap();
        assert!(
            matches!(knock(at).await, Knocked::Answer { refused: false, .. }),
            "a connection"
        );
        drop(listener);
        // Refused, or taken meanwhile by another test's listener: an answer either way.
        assert!(
            matches!(knock(at).await, Knocked::Answer { .. }),
            "a refusal"
        );
    });
}

#[test]
fn a_node_s_address_is_read_with_its_port() {
    assert_eq!(
        address("10.0.0.7:10250").unwrap().to_string(),
        "10.0.0.7:10250"
    );
    assert_eq!(
        address("fd00::7:10250").unwrap().to_string(),
        "[fd00::7]:10250"
    );
    assert_eq!(
        address("[fd00::7]:10250").unwrap().to_string(),
        "[fd00::7]:10250"
    );
    for wrong in ["", "10.0.0.7", "node:10250", "10.0.0.7:port", ":10250"] {
        assert!(address(wrong).is_err(), "{wrong}");
    }
}
