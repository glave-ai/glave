//! The real `glave` binary's turns in its own process, with no one at the terminal and no
//! server running: `--continue`, AGENTS.md, and Ctrl-C.

use std::process::Command;

use glave_provider_openai::fake::{FakeServer, Reply};

use crate::support::*;

#[test]
fn continue_picks_up_the_last_thread_and_a_new_run_starts_fresh() {
    let place = place("continue");
    let server = FakeServer::start(vec![
        Reply::text("Noted: marigold."),
        Reply::text("It was marigold."),
        Reply::text("Hello."),
    ])
    .unwrap();
    let first = glave(&place, &server, &["--model", "m", "remember marigold"], "");
    assert!(first.status.success(), "{}", text(&first));
    let second = glave(&place, &server, &["--model", "m", "-c", "what word?"], "");
    assert!(second.status.success(), "{}", text(&second));
    let third = glave(&place, &server, &["--model", "m", "hi"], "");
    assert!(third.status.success(), "{}", text(&third));
    let received = server.received();
    let resumed = &received[1].body;
    assert!(said(resumed, "user")[0].contains("remember marigold"));
    assert!(said(resumed, "assistant")[0].contains("Noted: marigold."));
    assert!(said(resumed, "user")[1].contains("what word?"));
    let fresh = &received[2].body;
    assert_eq!(said(fresh, "user").len(), 1, "a new thread: {fresh}");
}

#[test]
fn agents_md_from_the_project_and_the_person_reaches_the_model() {
    let place = place("agents-md");
    std::fs::write(place.project.join("AGENTS.md"), "Run make check.\n").unwrap();
    std::fs::create_dir_all(&place.home).unwrap();
    std::fs::write(place.home.join("AGENTS.md"), "Answer briefly.\n").unwrap();
    let server = FakeServer::start(vec![Reply::text("Hi.")]).unwrap();
    let output = glave(&place, &server, &["--model", "m", "hello"], "");
    let shown = text(&output);
    assert!(output.status.success(), "{shown}");
    let first = said(&server.received()[0].body, "user").join("\n");
    let order = [
        "# AGENTS.md instructions",
        "Answer briefly.",
        "Run make check.",
        "hello",
    ]
    .map(|text| first.find(text).unwrap_or(usize::MAX));
    assert!(
        order.windows(2).all(|pair| pair[0] < pair[1]),
        "the person's file, then the project's, then the prompt: {first}"
    );
    assert!(!said(&server.received()[0].body, "system")[0].contains("Run make check."));
}

#[cfg(unix)]
#[test]
fn ctrl_c_cancels_the_turn_and_the_thread_carries_on() {
    let place = place("ctrl-c");
    let server = FakeServer::start(vec![Reply::Stall, Reply::text("Still here.")]).unwrap();
    let events = place.project.join("../events.jsonl");
    let events_arg = events.to_string_lossy().into_owned();
    let child = start(
        &place,
        &server,
        &["--model", "m", "--events", &events_arg, "think hard"],
    );
    assert!(
        server.wait_for_requests(1, std::time::Duration::from_secs(30)),
        "the model was asked"
    );
    let sent = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(sent.success());
    let output = child.wait_with_output().unwrap();
    let shown = text(&output);
    assert_eq!(output.status.code(), Some(130), "{shown}");
    assert!(shown.contains("cancelled"), "{shown}");
    let recorded = std::fs::read_to_string(&events).unwrap();
    assert!(recorded.contains("\"outcome\":\"cancelled\""), "{recorded}");

    let output = glave(
        &place,
        &server,
        &["--model", "m", "--continue", "are you there?"],
        "",
    );
    let shown = text(&output);
    assert!(output.status.success(), "{shown}");
    assert!(shown.contains("Still here."), "{shown}");
    let asked = said(&server.received()[1].body, "user").join("\n");
    assert!(
        asked.contains("think hard") && asked.contains("cancelled this turn"),
        "the next turn knows the last was cancelled: {asked}"
    );
}
