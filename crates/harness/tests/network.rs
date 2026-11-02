//! Commands without the network (`specs/policy.md`, "The network"): one that needs it asks the
//! person to run with it, each time, and stays in the sandbox otherwise; with no one there it
//! never runs; where the sandbox has the network, nothing asks; and where the organization
//! forbids asking, no command asks, for the network or to run outside the sandbox, which has it.

use std::future::{Future, ready};
use std::net::TcpListener;
use std::path::Path;

use glave_event::{Body, Event, Lifted, Principal, StopReason, ThreadId, TurnResult, Verdict};
use glave_harness::{
    ApprovalRequest, Approver, Cancel, Decision, Defaults, HarnessError, NoOneToAsk,
};
use glave_ports::computer::Sandbox;
use glave_ports::{Caller, Completion, InMemoryLog, ModelRequest};
use serde_json::json;

use crate::outside_sandbox::{Person, decided, outside, sandboxed, walled};
use crate::support::*;

const REASON: &str = "it downloads the packages the project needs";
const CAUTION: &str = "It can reach the internet, which commands here otherwise cannot.";

/// A `bash` call that writes `file` beside the project, then connects to `listener` and says
/// so, asking for the network when `network`.
pub(crate) fn connect(
    base: &Path,
    file: &str,
    listener: &TcpListener,
    network: bool,
) -> (&'static str, serde_json::Value) {
    let port = listener.local_addr().unwrap().port();
    let command = format!(
        "echo x > {}; exec 3<>/dev/tcp/127.0.0.1/{port} && echo connected",
        base.join(file).display()
    );
    let arguments = if network {
        json!({ "command": command, "network": true, "reason": REASON })
    } else {
        json!({ "command": command })
    };
    ("bash", arguments)
}

/// The model's answers: each of `calls` in turn, then its last word.
pub(crate) fn making(calls: Vec<(&'static str, serde_json::Value)>) -> Vec<Completion> {
    let calls = calls
        .into_iter()
        .map(|call| answer(None, vec![call], StopReason::ToolUse));
    calls
        .chain([answer(Some("Done."), vec![], StopReason::EndTurn)])
        .collect()
}

/// What one turn did.
struct Ran {
    turn: TurnResult,
    /// What each call gave the model.
    said: Vec<String>,
    events: Vec<Event>,
    /// What the model was sent.
    seen: Vec<ModelRequest>,
}

impl Ran {
    /// Whether the model was offered `bash` with `parameter`.
    fn offers(&self, parameter: &str) -> bool {
        let bash = self
            .seen
            .first()
            .and_then(|request| request.tools.iter().find(|tool| tool.name == "bash"));
        bash.is_some_and(|bash| bash.parameters["properties"].get(parameter).is_some())
    }

    fn asked(&self) -> usize {
        let asked = |event: &&Event| matches!(event.body, Body::ApprovalRequested(_));
        self.events.iter().filter(asked).count()
    }
}

/// One turn of `answers` in a new thread on the project in `base`, its commands in the
/// sandbox with the `network` or without, and asking for it if `asks`, `approver` answering.
fn one_turn<A: Approver>(
    base: &Path,
    (network, asks): (bool, bool),
    answers: Vec<Completion>,
    approver: A,
) -> Ran {
    block_on!(async {
        let workspace = walled(base, network).await.asking_network(asks);
        let log = InMemoryLog::new();
        let (harness, seen) = harness_with(workspace, answers, approver, log, 1, in_the_sandbox());
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let turn = harness
            .run_turn(
                &mut thread,
                "install the packages",
                &Cancel::new(),
                &mut |_| {},
            )
            .await
            .unwrap();
        let events = events(harness.log(), thread.id).await;
        let seen = seen.lock().unwrap().clone();
        let said = results(thread.messages());
        Ran {
            turn,
            said,
            events,
            seen,
        }
    })
}

#[test]
fn a_command_without_the_network_asks_for_it_each_time_and_stays_in_the_sandbox_otherwise() {
    let Some(base) = sandboxed("network-allowed") else {
        return;
    };
    // A local listener only: no traffic leaves the machine.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let answers = making(vec![
        connect(&base, "a.txt", &listener, true),
        connect(&base, "b.txt", &listener, true),
        connect(&base, "c.txt", &listener, false),
    ]);
    // "For this thread" is the answer: the network is still allowed once each.
    let person = Person::answering(Decision::AllowForThread);
    let ran = one_turn(&base, (false, true), answers, person.clone());
    assert!(ran.offers("network"), "bash takes network here");
    let said = &ran.said;
    assert!(
        said[0].contains("connected") && said[1].contains("connected"),
        "{said:?}"
    );
    assert!(
        !said[2].contains("connected"),
        "the next is offline again: {said:?}"
    );
    for file in ["a.txt", "b.txt", "c.txt"] {
        assert!(
            !base.join(file).exists(),
            "{file} written beside the project: {said:?}"
        );
    }
    assert_eq!((ran.turn.with_network, ran.turn.outside_sandbox), (2, 0));
    let asked = person.asked();
    assert_eq!(asked.len(), 2, "each one asks");
    let request = asked[0].outside_sandbox.clone().unwrap();
    assert_eq!(
        (request.lifted, request.reason.as_deref()),
        (Lifted::Network, Some(REASON))
    );
    assert_eq!(
        decided(&ran.events),
        [(Verdict::AllowOnce, None), (Verdict::AllowOnce, None)]
    );
    let caution = ran.events.iter().find_map(|event| match &event.body {
        Body::ApprovalRequested(asked) => asked.caution.clone(),
        _ => None,
    });
    assert_eq!(
        caution.as_deref(),
        Some(CAUTION),
        "the model's reason kept apart"
    );
}

#[test]
fn with_no_one_there_a_command_never_gets_the_network() {
    let Some(base) = sandboxed("network-no-one") else {
        return;
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let answers = making(vec![connect(&base, "a.txt", &listener, true)]);
    let ran = one_turn(&base, (false, true), answers, NoOneToAsk);
    assert_eq!(ran.turn.with_network, 0);
    let said = &ran.said;
    assert!(
        said[0].starts_with("Not run, because it can reach the internet"),
        "{said:?}"
    );
    assert!(
        said[0].contains("Do not try to reach the internet another way"),
        "{said:?}"
    );
    assert_eq!(
        decided(&ran.events),
        [(Verdict::NoOneThere, Some(Principal::glave()))]
    );
}

#[test]
fn where_the_sandbox_has_the_network_asking_for_it_changes_nothing() {
    let Some(base) = sandboxed("network-on") else {
        return;
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let answers = making(vec![connect(&base, "a.txt", &listener, true)]);
    let ran = one_turn(&base, (true, true), answers, NoOneToAsk);
    assert!(!ran.offers("network"), "nothing to ask for");
    assert!(ran.said[0].contains("connected"), "{:?}", ran.said);
    assert_eq!((ran.turn.with_network, ran.asked()), (0, 0));
}

#[test]
fn where_the_organization_forbids_asking_no_command_asks_for_the_network_or_to_leave() {
    let Some(base) = sandboxed("network-forbidden") else {
        return;
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let answers = making(vec![
        connect(&base, "a.txt", &listener, true),
        outside(&base, "left.txt"),
    ]);
    let person = Person::answering(Decision::AllowOnce);
    let ran = one_turn(&base, (false, false), answers, person.clone());
    assert!(
        !ran.offers("network") && !ran.offers("outside_sandbox"),
        "the model is not told to ask"
    );
    assert!(person.asked().is_empty(), "never asked");
    for said in &ran.said {
        assert!(said.contains("is not allowed here"), "{:?}", ran.said);
        assert!(!said.contains("connected"), "{:?}", ran.said);
    }
    assert!(!base.join("left.txt").exists());
    assert_eq!((ran.turn.with_network, ran.turn.outside_sandbox), (0, 0));
    assert_eq!(ran.asked(), 0);
    // Where commands have the network, running outside the sandbox asks as ever.
    let answers = making(vec![outside(&base, "left.txt")]);
    let ran = one_turn(&base, (true, false), answers, person.clone());
    assert_eq!((person.asked().len(), ran.turn.outside_sandbox), (1, 1));
    assert!(base.join("left.txt").exists());
}

/// Waits for every answer in the log, as a server does.
pub(crate) struct InTheLog;

impl Approver for InTheLog {
    fn decide(&self, _request: &ApprovalRequest) -> impl Future<Output = Decision> + Send {
        ready(Decision::Unavailable)
    }

    fn can_answer(&self, _thread: ThreadId) -> bool {
        true
    }

    fn waits(&self, _thread: ThreadId) -> bool {
        true
    }
}

#[test]
fn an_answer_given_before_the_network_was_taken_never_gives_a_command_the_network() {
    let Some(base) = sandboxed("network-changed") else {
        return;
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    // Asked for its push, with the network on: nothing to let out then.
    let (tool, mut arguments) = connect(&base, "a.txt", &listener, true);
    let command = format!(
        "git push origin main; {}",
        arguments["command"].as_str().unwrap()
    );
    arguments["command"] = command.into();
    let answers = making(vec![(tool, arguments)]);
    block_on!(async {
        let workspace = walled(&base, true).await;
        let (harness, _) = harness_with(
            workspace,
            answers,
            InTheLog,
            InMemoryLog::new(),
            1,
            in_the_sandbox(),
        );
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let stopped = harness
            .run_turn(
                &mut thread,
                "install the packages",
                &Cancel::new(),
                &mut |_| {},
            )
            .await;
        assert!(
            matches!(stopped, Err(HarnessError::Waits(_))),
            "{stopped:?}"
        );
        let call = events(harness.log(), thread.id)
            .await
            .iter()
            .find_map(|event| match &event.body {
                Body::ApprovalRequested(asked) => Some(asked.call_id.clone()),
                _ => None,
            });
        // While it waits, the network is taken from commands.
        let mut defaults = Defaults::clone(&harness.defaults());
        defaults.sandbox = defaults.sandbox.map(|sandbox| Sandbox {
            network: false,
            ..sandbox
        });
        harness.set_defaults(defaults);
        let call = call.unwrap();
        harness
            .decide(scope(), thread.id, &call, Decision::AllowOnce, None)
            .await
            .unwrap();
        let turn = harness
            .go_on(&mut thread, &Caller::default(), &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        let said = results(thread.messages());
        assert!(said[0].contains("the sandbox changed"), "{said:?}");
        assert!(!said[0].contains("connected"), "{said:?}");
        assert_eq!(turn.with_network, 0);
    });
}
