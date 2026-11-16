use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::time::{Instant, sleep};

use super::{HOLD_WITHIN, KNOCK, Knocked, address, held_by, knock};

/// A runtime whose clock moves only when nothing else can run, so the knocks' waits and the
/// fence's limit take no real time.
fn paused() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap()
}

/// A knock that comes to `knocked`: a silence takes [`KNOCK`], as an unanswered one does.
async fn knocked(knocked: Knocked) -> Knocked {
    if knocked == Knocked::Silence {
        sleep(KNOCK).await;
    }
    knocked
}

/// Knocks that come to `script` in turn, then to its last again and again.
fn scripted(
    script: &'static [Knocked],
) -> impl FnMut() -> std::pin::Pin<Box<dyn Future<Output = Knocked>>> {
    let last = *script.last().unwrap();
    let mut script = script.iter().copied();
    move || Box::pin(knocked(script.next().unwrap_or(last)))
}

/// Knocks that come to `script` in turn, over and over.
fn cycled(script: &[Knocked]) -> impl FnMut() -> std::pin::Pin<Box<dyn Future<Output = Knocked>>> {
    let mut script = script.iter().copied().cycle();
    move || Box::pin(knocked(script.next().unwrap()))
}

fn node() -> SocketAddr {
    SocketAddr::from(([10, 0, 0, 7], 10250))
}

#[test]
fn three_unanswered_knocks_in_a_row_hold_the_fence() {
    use Knocked::{Answer, Silence};
    let knocks = paused().block_on(held_by(
        scripted(&[Answer, Answer, Silence, Silence, Silence]),
        node(),
        HOLD_WITHIN,
    ));
    assert_eq!(knocks.unwrap(), 5);
}

#[test]
fn an_answer_among_the_silences_starts_the_count_again() {
    use Knocked::{Answer, Silence};
    let knocks = paused().block_on(held_by(
        scripted(&[Silence, Silence, Answer, Silence, Silence, Silence]),
        node(),
        HOLD_WITHIN,
    ));
    assert_eq!(knocks.unwrap(), 6);
}

#[test]
fn a_node_that_answers_now_and_then_never_holds_it_and_says_where_and_what_next() {
    use Knocked::{Answer, Silence};
    // A slow node's answer reads as one silence; never as several in a row while it answers.
    let runtime = paused();
    let (said, took) = runtime.block_on(async {
        let started = Instant::now();
        let held = held_by(cycled(&[Silence, Silence, Answer]), node(), HOLD_WITHIN).await;
        (held.unwrap_err().to_string(), started.elapsed())
    });
    assert!(said.contains("10.0.0.7:10250"), "{said}");
    assert!(
        said.contains("isn't enforcing its network policy"),
        "{said}"
    );
    assert!(said.contains("next wake tries again"), "{said}");
    assert!(
        took >= HOLD_WITHIN && took < HOLD_WITHIN + Duration::from_secs(2),
        "it gives up at its limit: {took:?}"
    );
}

#[test]
fn a_knock_at_a_listener_or_a_closed_port_is_answered() {
    glave_ports::block_on!(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let at = listener.local_addr().unwrap();
        assert_eq!(knock(at).await, Knocked::Answer, "a connection");
        drop(listener);
        // Refused, or taken meanwhile by another test's listener: an answer either way.
        assert_eq!(knock(at).await, Knocked::Answer, "a refusal");
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
